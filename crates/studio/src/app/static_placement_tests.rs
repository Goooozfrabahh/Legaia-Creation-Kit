//! C1 headless acceptance. Runs native instruction slices, never an emulator.
use super::*;
use crate::app::{collision_recipes::digest, form_bridge::Session, lck_project::ProjectFile};

#[test]
fn standalone_mesh_negative_slope_and_overlap_roundtrip() -> Result<()> {
    let mut map = vec![0; MAP_LEN];
    // Different negative corners exercise native signed division, including
    // sums that are not multiples of four (arithmetic shift would drift).
    for row in 0..128 {
        for col in 0..128 {
            map[WALL_OFF + row * 128 + col] = ((row + col) % 4) as u8;
        }
    }
    let before = map.clone();
    let tiers = [
        Some(-1),
        Some(-2),
        Some(-3),
        Some(-5),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
        Some(0),
    ];
    let lut = [1, 2, 3, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let bindings = NativeBindings::parse(&map[0x10000..0x12000], &[])?;
    let mut ids = std::collections::BTreeSet::new();
    let mut cells = std::collections::BTreeSet::new();
    for _ in 0..3 {
        let (id, cell) = unrestricted::copy_static_mesh_record(
            &mut map,
            [400., 37., 400.],
            [400., 37., 400.],
            80.,
            &tiers,
            &bindings,
        )?;
        ensure!(
            ids.insert(id) && cells.insert(cell),
            "Overlapping References share native identity"
        );
        let parsed = field_objects::parse_walk_decorations(&map);
        let p = parsed
            .iter()
            .find(|p| p.obj_idx == id as u16)
            .context("Native decoration parse")?;
        ensure!(
            p.world_x == 400 && p.world_z == 400 && p.world_y(&lut) == -37,
            "Slope compensation drift"
        );
    }
    ensure!(
        map[WALL_OFF..GRID_OFF] == before[WALL_OFF..GRID_OFF]
            && map[0x10000..] == before[0x10000..],
        "Unowned sidecars changed"
    );
    Ok(())
}

#[test]
fn standalone_mesh_baked_coordinates_keep_visual_anchor_and_native_origin_separate() -> Result<()> {
    let mut map = vec![0; MAP_LEN];
    let bindings = NativeBindings::parse(&map[0x10000..0x12000], &[])?;
    let (id, cell) = unrestricted::copy_static_mesh_record(
        &mut map,
        [-64., 128., 128.],
        [4030.5, 352., 2624.5],
        80.,
        &[Some(0); 16],
        &bindings,
    )?;
    let r = ObjectRecord::parse(&map, id).context("record")?;
    ensure!(
        (cell.0 as i32 - 31).abs() <= 1 && (cell.1 as i32 - 20).abs() <= 1,
        "Anchor followed translation instead of visible geometry"
    );
    ensure!(
        field_objects::world_x(cell.0, r.x_off) == -64
            && field_objects::world_z(cell.1, r.z_off) == 128,
        "Baked geometry translation changed"
    );
    let mut staged = map.clone();
    let error = unrestricted::copy_static_mesh_record(
        &mut staged,
        [0., 0., 0.],
        [4032., 0., 2624.],
        80.,
        &[None; 16],
        &bindings,
    )
    .unwrap_err();
    ensure!(
        format!("{error:#}").contains("unresolved"),
        "Missing floor dependency not explicit: {error:#}"
    );
    Ok(())
}

#[test]
#[ignore = "retail standalone TMD, project and experimental IMG acceptance; no GUI"]
fn stage_c1_standalone_static_acceptance() -> Result<()> {
    let source = PathBuf::from(std::env::var_os("LCK_RETAIL_SOURCE").context("retail source")?);
    let output =
        PathBuf::from(std::env::var_os("LCK_C1_OUTPUT").context("new C1 output directory")?);
    ensure!(!output.exists(), "C1 evidence directory must be new");
    std::fs::create_dir(&output)?;
    let lib = crate::library::Library::open(&source)?;
    let target =
        legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index, "town01")
            .map_err(anyhow::Error::msg)?;
    let mut world = world_view::build_world(&target, &lib.index);
    let mut selected = None;
    // These are test search fixtures, not implementation eligibility rules.
    for name in ["bylon", "izumi", "town0b", "balden", "bubu1"] {
        let donor =
            legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index, name)
                .map_err(anyhow::Error::msg)?;
        let native = Scene::load(&lib.index, name)?;
        let context = super::super::native_asset_plan::static_resources::resolve_context(
            &lib.index, &native,
        )?;
        let bank = legaia_engine_scene::model_bank::SceneModelBank::build(&native);
        let models = bank.materialise(&native);
        for selector in 0..donor.env_tmds.len() {
            if donor
                .placements
                .iter()
                .chain(donor.terrain.iter())
                .any(|p| p.env_slot == selector)
            {
                continue;
            }
            let Some(model) = models.get(selector).and_then(|m| m.as_ref()) else {
                continue;
            };
            if context
                .dependencies(&native, selector as u16, model)
                .is_err()
            {
                continue;
            }
            let Ok((_, points)) = world_view::foreign_original_geometry(&donor, "mesh", selector)
            else {
                continue;
            };
            if points.is_empty() {
                continue;
            }
            let pivot = [0, 1, 2].map(|a| {
                let lo = points.iter().map(|p| p[a]).fold(f32::INFINITY, f32::min);
                let hi = points
                    .iter()
                    .map(|p| p[a])
                    .fold(f32::NEG_INFINITY, f32::max);
                (lo + hi) * 0.5
            });
            let center = [pivot[0] + 4032., pivot[1] + 128., pivot[2] + 2624.];
            world
                .insert_foreign(&donor, "mesh", selector, 7001, center, &lib.index)
                .map_err(anyhow::Error::msg)?;
            selected = Some((
                name.to_owned(),
                selector,
                pivot,
                digest(model),
                model.len(),
                native.field_map_index(&lib.index).context("donor MAP")?,
            ));
            break;
        }
        if selected.is_some() {
            break;
        }
    }
    let (name, selector, pivot, source_hash, source_size, donor_map) =
        selected.context("No eligible unplaced retail static TMD fixture")?;
    ensure!(
        world.foreign.len() == 1 && world.foreign[0].native_record.is_none(),
        "Standalone mesh unexpectedly has a donor record"
    );
    let mut session = Session::temporary(&source, &lib.index)?;
    session.enter(
        &target,
        &lib.index,
        &world,
        "original:town01".into(),
        "C1 acceptance".into(),
        None,
    )?;
    let original = session
        .project
        .legacy_bindings
        .iter()
        .find(|b| b.uid == Some(7001))
        .context("Imported Reference missing")?
        .reference
        .clone();
    let duplicate = session.duplicate(&[original.clone()])?.remove(0);
    session.transaction(|p| {
        p.database.edit_reference(&duplicate, |r| {
            r.transform.position = [pivot[0] + 4160., pivot[1] + 192., pivot[2] + 2624.];
            Ok(())
        })
    })?;
    let overlapping = session.duplicate(&[original.clone()])?.remove(0);
    let base = session
        .project
        .database
        .reference(&original)
        .unwrap()
        .base
        .clone();
    ensure!(
        original != duplicate && original != overlapping && duplicate != overlapping,
        "Reference identity reused"
    );
    for key in [&duplicate, &overlapping] {
        ensure!(
            session.project.database.reference(key).unwrap().base == base,
            "Duplicate did not share Base Form"
        );
    }
    let before = serde_json::to_vec(&session.project.database)?;
    let project_path = output.join("C1_STANDALONE.lckproject");
    ProjectFile::new(session.project.clone(), project_path.clone()).save()?;
    let reopened = ProjectFile::open(&project_path, Some(&session.source_fingerprint))?;
    ensure!(
        serde_json::to_vec(&reopened.project.database)? == before,
        "Project Reference save/reopen changed"
    );
    let cell = session.cell.clone();
    session.mount_project(reopened.project)?;
    session.cell = cell;
    let rendered = session.render(&target, &lib.index)?;
    ensure!(
        rendered.foreign.len() == 3,
        "Reopened standalone References missing"
    );
    let plan = prepare_project(&source, &target, &mut session, &lib.index, true)?;
    let coverage = plan.imports.as_ref().context("Coverage missing")?;
    ensure!(
        coverage.requested == 3 && coverage.written == 3,
        "C1 imports incomplete: {:?}",
        plan.skipped
    );
    let log = plan
        .unrestricted_log
        .as_ref()
        .context("Unrestricted log missing")?;
    ensure!(
        !log.lines().any(|l| l.starts_with("FAILED ")),
        "C1 writer failure: {log}"
    );
    let reports = plan
        .object_verification
        .iter()
        .filter(|r| r["category"] == "mesh")
        .cloned()
        .collect::<Vec<_>>();
    ensure!(
        reports.len() == 3 && reports.iter().all(|r| r["encoded_successfully"] == true),
        "C1 final identity report failed: {reports:?}"
    );
    let original_disc = DiscPatcher::open(std::fs::read(&source)?)?;
    let disc = DiscPatcher::open(plan.image.clone())?;
    let map_entry = Scene::load(&lib.index, "town01")?
        .field_map_index(&lib.index)
        .context("target MAP")? as usize;
    let old = original_disc.read_entry_footprint(map_entry)?;
    let map = disc.read_entry_footprint(map_entry)?;
    ensure!(
        map[0x4000..0x8000]
            .iter()
            .zip(&old[0x4000..0x8000])
            .all(|(&a, &b)| a & 15 == b & 15 && a & b == b)
            && map[0x10000..] == old[0x10000..],
        "Wall/floor/trigger bytes changed"
    );
    let mut ids = std::collections::BTreeSet::new();
    let mut cells = std::collections::BTreeSet::new();
    let mut selectors = std::collections::BTreeSet::new();
    let tiers = Scene::load(&lib.index, "town01")?
        .field_floor_height_lut(&lib.index)?
        .context("floor LUT")?;
    let placements = field_objects::parse_walk_decorations(&map);
    let mut origins = vec![];
    for r in &reports {
        let id = r["native_reference"]["descriptor"]
            .as_u64()
            .context("descriptor")? as usize;
        let xy = &r["native_reference"]["cell"];
        let cell = (xy[0].as_u64().unwrap() as u8, xy[1].as_u64().unwrap() as u8);
        ensure!(
            ids.insert(id) && cells.insert(cell),
            "Native descriptor/grid identity reused"
        );
        let rec = ObjectRecord::parse(&map, id).context("record")?;
        ensure!(rec.flags == 0x0012 && !rec.is_placed(), "C1 class changed");
        selectors.insert(rec.pack_index_field);
        let p = placements
            .iter()
            .find(|p| p.obj_idx == id as u16 && p.col == cell.0 && p.row == cell.1)
            .context("Independent MAP parser missing C1 placement")?;
        origins.push([p.world_x, -p.world_y(&tiers), p.world_z]);
    }
    origins.sort();
    let mut expected = vec![[4032, 128, 2624], [4032, 128, 2624], [4160, 192, 2624]];
    expected.sort();
    ensure!(
        origins == expected,
        "Native world transforms differ: {origins:?}"
    );
    ensure!(
        selectors.len() == 1,
        "Immutable resource registration duplicated"
    );
    let mut changed = vec![];
    for i in 0..disc.entry_count() {
        if disc.read_entry(i)? != original_disc.read_entry(i)? {
            changed.push(i);
            ensure!(
                [map_entry, map_entry + 3, map_entry + 4].contains(&i),
                "Unrelated PROT {i} changed"
            );
        }
    }
    ensure!(
        disc.read_entry(donor_map as usize)? == original_disc.read_entry(donor_map as usize)?,
        "Donor MAP changed"
    );
    // Execute exact original non-GTE floor and selector instruction slices.
    let native900 = lib.index.entry_bytes(900)?;
    let mut cpu = legaia_code_hooks::mips_sim::Cpu::new();
    cpu.load_delays = true;
    cpu.strict_alignment = true;
    cpu.load(0x801f69d8, &native900);
    for report in &reports {
        let cell = &report["native_reference"]["cell"];
        let center = &report["requested_editor_center"];
        let col = (center[0].as_f64().context("center X")? / 128.).floor() as u8;
        let row = (center[2].as_f64().context("center Z")? / 128.).floor() as u8;
        cpu.r[17] = 0x80400000;
        cpu.wr32(0x80400014, 0);
        cpu.wr32(0x80400018, 0);
        cpu.r[18] = cell[0].as_u64().unwrap() as u32;
        cpu.r[19] = cell[1].as_u64().unwrap() as u32;
        for (at, value) in [
            (0x1f800384, col - 2),
            (0x1f800386, col + 3),
            (0x1f800385, row - 2),
            (0x1f800387, row + 3),
        ] {
            cpu.wr8(at, value);
        }
        cpu.pc = 0x801f74dc;
        while ![0x801f7544, 0x801f7548, 0x801f78d4].contains(&cpu.pc) {
            ensure!(cpu.steps < 1000, "Native window gate step bound");
            let w = cpu.rd32(cpu.pc);
            cpu.exec(w);
        }
        ensure!(
            cpu.pc != 0x801f78d4,
            "Original camera-window gate skipped C1 anchor near requested visible center"
        );
    }
    for sum in [-7i32, -4, -1, 0, 1, 7] {
        cpu.r[2] = sum as u32;
        cpu.r[5] = 0;
        cpu.r[3] = 0;
        cpu.r[17] = 0x80400000;
        cpu.pc = 0x801f76cc;
        while cpu.pc != 0x801f76e4 {
            ensure!(cpu.steps < 1000, "Native slice step bound");
            let w = cpu.rd32(cpu.pc);
            cpu.exec(w);
        }
        ensure!(
            cpu.rd16(0x8040001e) as i16 == (sum / 4) as i16,
            "Original native floor rounding mismatch"
        );
    }
    let selected = *selectors.iter().next().unwrap();
    cpu.r[16] = 0x80400100;
    cpu.wr16(0x80400110, selected);
    cpu.wr32(0x8007b6f8, 5);
    cpu.wr32(0x8007c018 + (selected as u32 + 5) * 4, 0x80410000);
    cpu.pc = 0x801f77e4;
    while cpu.pc != 0x801f780c {
        ensure!(cpu.steps < 1000, "Native slice step bound");
        let w = cpu.rd32(cpu.pc);
        cpu.exec(w);
    }
    ensure!(
        cpu.r[2] == 0x80410000,
        "Original decoration selector lookup differs"
    );
    let evidence = serde_json::json!({"donor_scene":name,"donor_selector":selector,"source_tmd_sha256":source_hash,"source_tmd_bytes":source_size,"donor_has_MAP_placement":false,"references":[original,duplicate,overlapping],"native_origins":origins,"native_selectors":selectors,"changed_PROT_entries":changed,"native_instruction_slices":["801f76cc..801f76e4","801f77e4..801f780c"],"per_object":reports,"gameplay_verified":false});
    std::fs::write(
        output.join("C1_NATIVE_ACCEPTANCE.json"),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    drop(disc);
    drop(original_disc);
    let img = output.join("LCK_EXPERIMENTAL_C1_STANDALONE_STATIC.img");
    let result = export(plan, &img)?;
    ensure!(
        !result.skipped.iter().any(|s| s.starts_with("FAILED ")),
        "Published C1 failures: {:?}",
        result.skipped
    );
    println!("C1 acceptance: {evidence}; IMG {}", img.display());
    Ok(())
}
