use super::super::form_database::{FileId, FormDatabase, FormKind, Property};
use super::*;
fn key(n: u32) -> FormKey {
    FormKey::new(FileId::new("test").unwrap(), n).unwrap()
}
#[test]
fn actor_records_have_independent_contexts_and_native_transforms() -> Result<()> {
    let original = super::super::native_script_dependencies::fixture_man(&[&[0x21], &[0x21]]);
    let t = Transform {
        position: [64., 12., 192.],
        rotation: [0., std::f32::consts::FRAC_PI_2, 0.],
        scale: [1.25; 3],
    };
    let (bytes, seats) = append_actors(
        &original,
        &[
            (key(1), key(10), 4, 7, t),
            (
                key(2),
                key(10),
                4,
                7,
                Transform {
                    position: [128., 12., 256.],
                    ..t
                },
            ),
        ],
    )?;
    assert_eq!(
        seats.iter().map(|s| s.context).collect::<Vec<_>>(),
        vec![2, 3]
    );
    let old = man_section::parse(&original).map_err(anyhow::Error::msg)?;
    let new = man_section::parse(&bytes).map_err(anyhow::Error::msg)?;
    for i in 0..2 {
        let a = old.actor_placement(&original, i).unwrap();
        let b = new.actor_placement(&bytes, i).unwrap();
        assert_eq!(
            &original[a.record_offset..a.record_offset + 6],
            &bytes[b.record_offset..b.record_offset + 6]
        );
    }
    let inv = super::super::native_script_dependencies::SceneInventory::read(&bytes)
        .map_err(anyhow::Error::msg)?;
    assert!(inv.gaps.is_empty(), "{:?}", inv.gaps);
    assert!(
        append_actors(
            &original,
            &[(key(1), key(10), 4, 7, t), (key(1), key(10), 4, 7, t)]
        )
        .is_err()
    );
    assert!(actor_record(240, 1, &t).is_err());
    assert!(
        actor_record(
            1,
            1,
            &Transform {
                position: [65., 0., 64.],
                ..t
            }
        )
        .is_err()
    );
    assert!(
        actor_record(
            1,
            1,
            &Transform {
                scale: [1., 2., 1.],
                ..t
            }
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn reference_animation_state_survives_duplicate_move_delete_and_project_records() -> Result<()> {
    let mut d = FormDatabase::new(FileId::new("retail")?, FileId::new("test")?)?;
    let (b, c) = d.transaction(|d| {
        Ok((
            d.create_base("Actor".into(), FormKind::Actor)?,
            d.create_base("Cell".into(), FormKind::Cell)?,
        ))
    })?;
    let a = d.transaction(|d| d.place_reference(b, c, Transform::default()))?;
    d.transaction(|d| {
        d.edit_reference(&a, |r| {
            r.properties
                .insert("npc_animation_clip".into(), Property::Integer(2));
            Ok(())
        })
    })?;
    let b = d.transaction(|d| d.duplicate_reference(&a))?;
    d.transaction(|d| {
        d.edit_reference(&b, |r| {
            r.transform.position = [64., 0., 192.];
            r.properties
                .insert("npc_animation_clip".into(), Property::Integer(3));
            Ok(())
        })
    })?;
    assert_eq!(
        d.reference(&a).unwrap().properties["npc_animation_clip"],
        Property::Integer(2)
    );
    d.transaction(|d| d.delete_reference(&a))?;
    let mut restored: FormDatabase = serde_json::from_slice(&serde_json::to_vec(&d)?)?;
    restored.validate()?;
    assert!(restored.reference(&a).unwrap().deleted);
    assert!(
        restored
            .reference(&b)
            .unwrap()
            .flags
            .contains("native_animated_reference")
    );
    assert_eq!(
        restored.reference(&b).unwrap().transform.position,
        [64., 0., 192.]
    );
    Ok(())
}
/// Original SCUS clip binding/ticking; no host animation resolver substitutes it.
fn original_clip_driver(exe: &[u8], bank: &[u8], id: u8) -> Result<(u32, u16)> {
    use legaia_code_hooks::mips_sim::Cpu;
    let load = u32::from_le_bytes(exe[0x18..0x1c].try_into()?);
    let mut cpu = Cpu::new();
    cpu.load_delays = true;
    cpu.strict_alignment = true;
    cpu.load(load, &exe[0x800..]);
    let bank_addr = 0x80400000;
    let actor = 0x80500000;
    cpu.load(bank_addr, bank);
    cpu.wr32(0x8007b888, bank_addr);
    cpu.wr32(actor + 0x10, 0x20000);
    cpu.wr16(actor + 0x5c, id as u16);
    cpu.wr16(actor + 0x5e, 0xfffe);
    cpu.wr16(actor + 0x72, 0x1000);
    cpu.wr16(actor + 0x68, 0x120);
    cpu.r[4] = actor;
    cpu.r[29] = 0x807ff000;
    cpu.r[28] = 0x8007b318;
    cpu.r[31] = 0x8ffffffc;
    cpu.pc = 0x800204f8;
    while cpu.pc != 0x8ffffffc {
        ensure!(
            cpu.steps < 10000,
            "Native clip-driver step budget at {:#x}",
            cpu.pc
        );
        ensure!(
            (0x800204f8..0x80020900).contains(&cpu.pc),
            "Unexpected native clip-driver helper at {:#x}",
            cpu.pc
        );
        let w = cpu.rd32(cpu.pc);
        cpu.exec(w);
    }
    ensure!(
        cpu.rd16(actor + 0x68) == 0,
        "Original clip rebind did not reset initial frame"
    );
    Ok((cpu.rd32(actor + 0x4c), cpu.rd16(actor + 0x5e)))
}
/// Native installer + materializer instructions. Node/heap allocation, the GTE helper and RNG
/// are substituted; stop before the overlay's spawn-prologue dispatcher.
fn original_actor_installer(
    exe: &[u8],
    man: &[u8],
    bank: &[u8],
    p1: usize,
    model_bytes: &[u8],
    prior: Option<legaia_code_hooks::mips_sim::Cpu>,
) -> Result<(u16, u16, u32, legaia_code_hooks::mips_sim::Cpu)> {
    use legaia_code_hooks::mips_sim::Cpu;
    let load = u32::from_le_bytes(exe[0x18..0x1c].try_into()?);
    let fresh=prior.is_none();
    let mut cpu = prior.unwrap_or_default();
    cpu.load_delays = true;
    cpu.strict_alignment = true;
    if fresh {cpu.load(load, &exe[0x800..]);}
    let man_addr = 0x80400000;
    let bank_addr = 0x80500000;
    let actor = 0x80600000+p1 as u32*0x100;
    cpu.load(man_addr, man);
    cpu.load(bank_addr, bank);
    cpu.wr32(0x8007b898, man_addr);
    cpu.wr32(0x8007b888, bank_addr);
    cpu.wr16(0x8007b6f8, 5);
    let mf = man_section::parse(man).map_err(anyhow::Error::msg)?;
    let placement = mf.actor_placement(man, p1).context("Actor record")?;
    let model_address=0x80800000+u32::from(placement.model_index)*0x10000;
    cpu.load(model_address, model_bytes);
    cpu.wr32(
        0x8007c018 + (u32::from(placement.model_index) + 5) * 4,
        model_address,
    );
    cpu.wr32(0x8007bb38, cpu.rd32(0x8007bb38).max(u32::from(placement.model_index) + 5));
    let mut heap = 0x81800000u32+p1 as u32*0x10000;
    cpu.r[4] = p1 as u32;
    cpu.r[5] = mf.partitions[0].len() as u32;
    cpu.r[29] = 0x807ff000;
    cpu.r[28] = 0x8007b318;
    cpu.r[31] = 0x8ffffffc;
    cpu.pc = 0x8003a1e4;
    let mut allocations = 0;
    while cpu.pc != 0x8003a474 {
        ensure!(
            cpu.steps < 30000,
            "Native actor install instruction budget at {:#x}",
            cpu.pc
        );
        match cpu.pc {
            0x80020454 => {
                allocations += 1;
                ensure!(
                    allocations == 1,
                    "More than one native actor node allocation requested"
                );
                cpu.r[2] = actor;
                cpu.pc = cpu.r[31];
            }
            0x80017888 => {
                let size = cpu.r[5];
                ensure!(size < 0x10000, "Unexpected native actor allocation extent");
                let ptr = heap;
                heap += (size + 31) & !31;
                cpu.r[2] = ptr;
                cpu.pc = cpu.r[31];
            }
            0x8003d344 => {
                let out = cpu.r[5];
                for i in 0..3 {
                    cpu.wr32(out + i * 4, 0);
                }
                cpu.pc = cpu.r[31];
            }
            0x80056798 => {
                cpu.r[2] = 0;
                cpu.pc = cpu.r[31];
            }
            pc => {
                ensure!(
                    pc >= load && ((pc - load) as usize) < exe.len() - 0x800,
                    "Unsubstituted native installer helper {pc:#x}; RA {:#x}, sp {:#x}, a0 {:#x}, v0 {:#x}",
                    cpu.r[31],
                    cpu.r[29],
                    cpu.r[4],
                    cpu.r[2]
                );
                let word = cpu.rd32(pc);
                cpu.exec(word);
            }
        }
    }
    ensure!(
        allocations == 1,
        "Original installer did not allocate one actor"
    );
    let p = mf.actor_placement(man, p1).context("MAN actor")?;
    ensure!(
        cpu.rd16(actor + 0x50) as usize == mf.partitions[0].len() + p1,
        "Original installer chose wrong context ID"
    );
    ensure!(
        cpu.rd16(actor + 0x14) == p.world_x as u16 && cpu.rd16(actor + 0x18) == p.world_z as u16,
        "Original materializer chose wrong position"
    );
    ensure!(
        cpu.rd32(actor + 0x10) & 0x01000000 == 0,
        "Scene actor acquired party-bank flag"
    );
    let objects = cpu.rd32(actor + 0x44);
    let count = u32::from_le_bytes(model_bytes[8..12].try_into()?);
    ensure!(
        objects != 0 && cpu.rd32(objects) == count,
        "Original native object-table allocation differs from model count"
    );
    ensure!(
        cpu.rd16(actor + 0x56) == 1,
        "Original ANM clip driver did not select animated draw class"
    );
    Ok((
        cpu.rd16(actor + 0x64),
        cpu.rd16(actor + 0x5c),
        cpu.rd32(actor + 0x4c),
        cpu,
    ))
}
#[test]
#[ignore = "retail ANM/model portability, native SCUS clip driver and in-memory IMG resource serialization"]
fn stage_b2b_native_animated_portability() -> Result<()> {
    use super::super::{form_bridge::Session, world_view};
    let source =
        std::path::PathBuf::from(std::env::var_os("LCK_AUDIT_SOURCE").context("retail source")?);
    let report =
        std::path::PathBuf::from(std::env::var_os("LCK_B2B_REPORT").context("new report path")?);
    ensure!(!report.exists(), "Report must use a new path");
    let lib = crate::library::Library::open(&source)?;
    let exe = lib.executable()?;
    let mut candidates = vec![];
    let mut unsupported = vec![];
    for name in ["balden", "bubu1", "town0b", "bylon"] {
        let scene = Scene::load(&lib.index, name)?;
        let man = scene.field_man_payload(&lib.index)?.context("MAN")?;
        let mf = man_section::parse(&man).map_err(anyhow::Error::msg)?;
        for actor in mf.actor_placements(&man) {
            match resolve(&lib.index, name, actor.index, None) {
                Ok(c) => {
                    let mut bank = vec![1, 0, 0, 0, 8, 0, 0, 0];
                    bank.extend(&c.clip);
                    let (ptr, frame) = original_clip_driver(&exe, &bank, 1)?;
                    ensure!(
                        ptr == 0x80400008,
                        "Original native clip driver selected wrong ANM pointer: {ptr:#x}, frame {frame}"
                    );
                    println!(
                        "DONOR {name} P1[{}] {} objects ANM {} bytes original clip pointer matched",
                        actor.index,
                        legaia_tmd::parse(&c.model)?.objects.len(),
                        c.clip.len()
                    );
                    candidates.push((name.to_string(), actor.index, c.pivot));
                    break;
                }
                Err(e) => unsupported.push(format!("{name}:{}: {e:#}", actor.index)),
            }
        }
        if candidates.len() == 2 {
            break;
        }
    }
    ensure!(candidates.len() == 2, "Two native donor families required");
    let target =
        legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index, "town01")
            .map_err(anyhow::Error::msg)?;
    let mut session = Session::temporary(&source, &lib.index)?;
    for (name, _, _) in &candidates {
        let s = legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index, name)
            .map_err(anyhow::Error::msg)?;
        let w = world_view::build_world(&s, &lib.index);
        session.enter(
            &s,
            &lib.index,
            &w,
            format!("original:{name}"),
            name.clone(),
            None,
        )?;
    }
    let w = world_view::build_world(&target, &lib.index);
    session.enter(
        &target,
        &lib.index,
        &w,
        "original:town01".into(),
        "town01".into(),
        None,
    )?;
    let cell = session.cell.clone().unwrap();
    let mut added = vec![];
    for (i, (name, actor, pivot)) in candidates.iter().enumerate() {
        let base = session
            .project
            .base_adapters
            .iter()
            .find(|(_, a)| a.source_scene == *name && a.kind == "npc" && a.source_ordinal == *actor)
            .map(|(k, _)| k.clone())
            .context("Actor Base Form not catalogued")?;
        let r = session.project.database.transaction(|d| {
            d.place_reference(
                base,
                cell.clone(),
                Transform {
                    position: [
                        4800. + i as f32 * 128. + pivot[0],
                        pivot[1],
                        2368. + pivot[2],
                    ],
                    ..Default::default()
                },
            )
        })?;
        added.push(r);
    }
    let duplicate = session
        .project
        .database
        .transaction(|d| d.duplicate_reference(&added[0]))?;
    session.project.database.transaction(|d| {
        d.edit_reference(&duplicate, |r| {
            r.transform.position = [
                5056. + candidates[0].2[0],
                candidates[0].2[1],
                2496. + candidates[0].2[2],
            ];
            Ok(())
        })
    })?;
    let snapshot = serde_json::to_vec(&session.project)?;
    session.project = serde_json::from_slice(&snapshot)?;
    session.project.validate()?;
    let disc = legaia_disc_patch::disc::DiscPatcher::open(std::fs::read(&source)?)?;
    let (output, compiled) = compile_project(disc, &session, &lib.index)?;
    ensure!(
        compiled.seats.len() == 3 && compiled.seats[0].reference != compiled.seats[1].reference,
        "Independent native references missing"
    );
    ensure!(
        compiled
            .seats
            .iter()
            .filter(|s| s.base == compiled.seats[0].base)
            .map(|s| (s.model, s.animation))
            .collect::<BTreeSet<_>>()
            .len()
            == 1,
        "Shared immutable resources registered twice"
    );
    let map = Scene::load(&lib.index, "town01")?
        .field_map_index(&lib.index)
        .unwrap() as usize;
    ensure!(
        output.read_entry_footprint(map)? == lib.index.entry_bytes(map as u32)?.as_slice(),
        "Animation modified unrelated MAP/walls"
    );
    let bundle = output.read_entry_footprint(map + 3)?;
    let (base, header) = reg::native_descriptor_table(&bundle)?;
    let d = header
        .used()
        .iter()
        .find(|d| d.type_byte == 5)
        .context("ANM")?;
    let bank = legaia_lzs::decompress(&bundle[base + d.data_offset as usize..], d.size as usize)?;
    let md = header
        .used()
        .iter()
        .find(|d| d.type_byte == 3)
        .context("MAN")?;
    let native_man =
        legaia_lzs::decompress(&bundle[base + md.data_offset as usize..], md.size as usize)?;
    let pac = reg::native_stream_mode(&bundle)
        .map(|_| output.read_entry_footprint(map + 4))
        .transpose()?;
    let native_trace = super::super::native_loader_trace::trace(&bundle, base, pac.as_deref())?;
    let (original_models, allocations) =
        super::super::native_loader_trace::tests::execute_original_allocations(
            &exe,
            &bundle,
            base,
            pac.as_deref(),
            true,
        )?;
    ensure!(
        original_models.len() == native_trace.models.len()
            && original_models
                .iter()
                .zip(&native_trace.models)
                .all(|((slot, hash), m)| *slot == m.global_slot && *hash == m.sha256),
        "Original SCUS registered different model identities"
    );
    let parsed = player_anm::parse(&bank)?;
    let mut native_machine=None;
    for s in &compiled.seats {
        let (model, clip, ptr, machine) = original_actor_installer(
            &exe,
            &native_man,
            &bank,
            s.p1,
            &native_trace.models[s.model as usize].bytes,
            native_machine.take(),
        )?;
        native_machine=Some(machine);
        ensure!(
            model == u16::from(s.model) + 5
                && clip == u16::from(s.animation)
                && ptr == 0x80500000 + parsed.record_offsets[s.animation as usize - 1],
            "Original actor installer registration differs: model {model}, clip {clip}, pointer {ptr:#x}"
        );
        let (ptr, _) = original_clip_driver(&exe, &bank, s.animation)?;
        ensure!(
            ptr == 0x80400000 + parsed.record_offsets[s.animation as usize - 1],
            "Native ANM registration differs after packaging"
        );
    }
    let mut machine=native_machine.context("Native actor state machine missing")?;
    let actors=compiled.seats.iter().map(|s|0x80600000+s.p1 as u32*0x100).collect::<Vec<_>>();
    let old_other=actors[1..].iter().map(|a|(machine.rd16(a+0x5c),machine.rd32(a+0x4c),machine.rd16(a+0x68))).collect::<Vec<_>>();
    machine.wr16(actors[0]+0x5c,compiled.seats[1].animation as u16);machine.wr16(actors[0]+0x5e,0xfffe);machine.wr16(actors[0]+0x68,0x120);
    machine.r[4]=actors[0];machine.r[29]=0x807ff000;machine.r[31]=0x8ffffffc;machine.pc=0x800204f8;
    while machine.pc!=0x8ffffffc {ensure!(machine.steps<50000,"Independent native clip tick budget");ensure!((0x800204f8..0x80020900).contains(&machine.pc),"Unexpected native clip helper");let w=machine.rd32(machine.pc);machine.exec(w);}
    ensure!(machine.rd16(actors[0]+0x68)==0,"Independent native clip rebind failed");
    for (a,state) in actors[1..].iter().zip(&old_other) {ensure!((machine.rd16(a+0x5c),machine.rd32(a+0x4c),machine.rd16(a+0x68))==*state,"Changing one native actor mutated another animation state");}
    let prepared = super::super::ps1_export::prepare_project(
        &source,
        &target,
        &mut session,
        &lib.index,
        true,
    )?;
    let (bytes, reports, failures) = prepared.animation_test_inspection();
    ensure!(
        !failures.iter().any(|s| s.starts_with("FAILED")),
        "Project animated export failures: {failures:?}"
    );
    ensure!(
        reports
            .iter()
            .filter(|r| r["native_animation"]["status"] == "encoded native animated Reference")
            .count()
            == 3,
        "Per-Reference project compiler results missing"
    );
    legaia_disc_patch::disc::DiscPatcher::open(bytes.to_vec())?;
    if let Some(path) = std::env::var_os("LCK_B2B_OUTPUT") {
        let root = std::path::PathBuf::from(path);
        ensure!(
            !root.exists(),
            "Experimental output requires a new directory"
        );
        std::fs::create_dir_all(&root)?;
        let mut file = super::super::lck_project::ProjectFile::new(
            session.project.clone(),
            root.join("B2B_Animated_References.lckproject"),
        );
        file.save()?;
        let reread = super::super::lck_project::ProjectFile::open(
            &file.path,
            Some(&session.source_fingerprint),
        )?;
        ensure!(
            serde_json::to_vec(&reread.project.database)?
                == serde_json::to_vec(&session.project.database)?,
            "Saved project changed animated identities/state"
        );
        let output_path = root.join("LCK_EXPERIMENTAL_B2B_NATIVE_ANIMATION.img");
        let exported = super::super::ps1_export::export(prepared, &output_path)?;
        ensure!(
            !exported.skipped.iter().any(|s| s.starts_with("FAILED")),
            "Published animated output has unresolved writes: {:?}",
            exported.skipped
        );
        let reopened = crate::library::Library::open(&output_path)?;
        let native = Scene::load(&reopened.index, "town01")?;
        let man = native
            .field_man_payload(&reopened.index)?
            .context("Native MAN reopen failed")?;
        let mf = man_section::parse(&man).map_err(anyhow::Error::msg)?;
        for seat in &compiled.seats {
            let p = mf
                .actor_placement(&man, seat.p1)
                .context("Exported animated actor disappeared on reopen")?;
            ensure!(
                p.model_index == seat.model && p.anim_id == seat.animation,
                "Exported actor identity changed on reopen"
            );
        }
        println!(
            "EXPERIMENTAL animated IMG and project retained at {} (manual GUI/gameplay unverified)",
            root.display()
        );
    }
    session
        .project
        .database
        .transaction(|d| d.delete_reference(&duplicate))?;
    let (_, after_delete) = compile_project(
        legaia_disc_patch::disc::DiscPatcher::open(std::fs::read(&source)?)?,
        &session,
        &lib.index,
    )?;
    ensure!(
        after_delete.seats.len() == 2,
        "Deleted Reference retained native allocation"
    );
    for r in &added {
        session
            .project
            .database
            .transaction(|d| d.delete_reference(r))?;
    }
    let (empty, retired) = compile_project(
        legaia_disc_patch::disc::DiscPatcher::open(std::fs::read(&source)?)?,
        &session,
        &lib.index,
    )?;
    ensure!(
        retired.seats.is_empty() && digest(empty.image()) == digest(&std::fs::read(&source)?),
        "Last-owner deletion retained added resources"
    );
    std::fs::write(
        &report,
        serde_json::to_vec_pretty(
            &serde_json::json!({"compilation":compiled,"after_delete":after_delete,"unsupported":unsupported,"original_clip_driver_verified":true,"original_model_registration_verified":true,"original_actor_installer_verified":true,"native_loader_allocations":allocations,"GUI_verified":false,"gameplay_verified":false}),
        )?,
    )?;
    println!(
        "B2B native animation resource/reference readback passed; report {}",
        report.display()
    );
    Ok(())
}

#[test]
#[ignore = "original SCUS actor installer with synthetic actor allocation and deterministic RNG"]
fn stage_b2b_original_installer_probe() -> Result<()> {
    let source = std::path::PathBuf::from(std::env::var_os("LCK_AUDIT_SOURCE").context("source")?);
    let lib = crate::library::Library::open(&source)?;
    let c = resolve(&lib.index, "balden", 1, None)?;
    let mut bank = vec![1, 0, 0, 0, 8, 0, 0, 0];
    bank.extend(c.clip);
    let original = super::super::native_script_dependencies::fixture_man(&[&[0x21], &[0x21]]);
    let (man, seats) = append_actors(
        &original,
        &[(
            key(1),
            key(2),
            4,
            1,
            Transform {
                position: [64., 0., 192.],
                ..Default::default()
            },
        )],
    )?;
    let (model, clip, ptr, _) =
        original_actor_installer(&lib.executable()?, &man, &bank, seats[0].p1, &c.model, None)?;
    ensure!(
        model == 9 && clip == 1 && ptr == 0x80500008,
        "Original actor model/clip pointer differs: model {model}, clip {clip}, ptr {ptr:#x}"
    );
    println!(
        "Original SCUS MAN actor installer: one allocation, native model global slot {model}, independent context {}, exact clip pointer {ptr:#x}; synthetic node/heap, GTE and RNG; no gameplay",
        seats[0].context
    );
    Ok(())
}

#[test]
fn clip_banks_share_exact_payloads_and_preserve_existing_indices() -> Result<()> {
    let mut record = vec![1, 0, 1, 0, 0x0c, 8, 2, 0];
    record.extend([0; 16]);
    let mut bank = vec![1, 0, 0, 0, 8, 0, 0, 0];
    bank.extend(&record);
    let mut changed = record.clone();
    changed[8] = 3;
    let (output, ids) = append_clips(&bank, &[record.clone(), changed.clone(), changed.clone()])?;
    assert_eq!(ids, vec![1, 2, 2]);
    let parsed = player_anm::parse(&output)?;
    assert_eq!(parsed.record_count, 2);
    assert_eq!(parsed.record_bytes(0), record);
    assert_eq!(parsed.record_bytes(1), changed);
    assert!(append_clips(&bank, &[vec![0; 3]]).is_err());
    Ok(())
}

/// Continue the real installer through its previously untested spawn pre-run.
/// Allocation/GTE remain substituted by original_actor_installer; this proves
/// bytecode execution only, never live heap or gameplay compatibility.
fn original_spawn_prerun(cpu: &mut legaia_code_hooks::mips_sim::Cpu, overlay: &[u8]) -> Result<()> {
    cpu.load(0x801ce818, overlay);
    let start=cpu.steps;
    while cpu.pc != 0x8003a4fc {
        ensure!(cpu.steps-start<100000,"Spawn pre-run stuck at {:#x}",cpu.pc);
        ensure!((0x80010000..0x8007b318).contains(&cpu.pc)||(0x801ce818..0x801ce818+overlay.len() as u32).contains(&cpu.pc),"Spawn pre-run escaped at {:#x}",cpu.pc);
        let w=cpu.rd32(cpu.pc);cpu.exec(w);
    }
    Ok(())
}

#[test]
#[ignore="read-only failed IMG instruction-level startup diagnosis; no emulator"]
fn stage_b2b_failed_image_startup_probe() -> Result<()> {
    let lib=crate::library::Library::open(std::path::Path::new(&std::env::var_os("LCK_FAILED_IMG").context("failed IMG")?))?;
    let exe=lib.executable()?;let scene=Scene::load(&lib.index,"town01")?;
    let man=scene.field_man_payload(&lib.index)?.context("MAN")?;
    let bank=legaia_engine_scene::npc_catalog::scene_anm_bundle(&scene).context("ANM")?;
    let overlay=lib.index.entry_bytes(897)?;
    let context=assets::static_resources::resolve_actor_context(&lib.index,&scene)?;
    let mf=man_section::parse(&man).map_err(anyhow::Error::msg)?;
    for p in mf.actor_placements(&man).into_iter().filter(|p|p.index>=53) {
        let (_,_,_,mut cpu)=original_actor_installer(&exe,&man,&bank.decoded,p.index,context.models[p.model_index as usize].as_ref().context("model")?,None)?;
        original_spawn_prerun(&mut cpu,&overlay)?;
        println!("Completed native spawn pre-run P1[{}], PC {}",p.index,cpu.rd16(0x80600000+p.index as u32*0x100+0x9e));
    }
    Ok(())
}

#[test]
#[ignore="clean-retail isolated IMG matrix, exact failed project and native loader evidence; no GUI"]
fn stage_b2b_loading_isolation_matrix() -> Result<()> {
    use super::super::{form_bridge::Session, world_view, lck_project::ProjectFile, ps1_export};
    let source=std::path::PathBuf::from(std::env::var_os("LCK_AUDIT_SOURCE").context("retail")?);
    let failed=std::path::PathBuf::from(std::env::var_os("LCK_FAILED_IMG").context("failed")?);
    let project=std::path::PathBuf::from(std::env::var_os("LCK_FAILED_PROJECT").context("project")?);
    let root=std::path::PathBuf::from(std::env::var_os("LCK_ISOLATION_OUTPUT").context("new output")?);
    ensure!(!root.exists(),"Isolation evidence must use a new directory");std::fs::create_dir_all(&root)?;
    let lib=crate::library::Library::open(&source)?;
    let target=legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index,"town01").map_err(anyhow::Error::msg)?;
    let mut session=Session::temporary(&source,&lib.index)?;
    session.enter(&target,&lib.index,&world_view::build_world(&target,&lib.index),"original:town01".into(),"town01".into(),None)?;
    let saved=ProjectFile::open(&project,Some(&session.source_fingerprint))?;
    session.project=saved.project;
    let keys=session.project.database.references().values().filter(|r|!r.deleted&&r.flags.contains("native_animated_reference")).map(|r|r.key.clone()).collect::<Vec<_>>();
    ensure!(keys.len()==3,"Expected exact original three-reference failure arrangement, found {}",keys.len());
    session.cell=Some(session.project.database.reference(&keys[0]).unwrap().cell.clone());
    let original=session.project.clone();let exe=lib.executable()?;
    let mut rows=vec![];
    for(name,keep)in [("A_CONTROL",vec![]),("B_FIRST_BALDEN",vec![0]),("C_SECOND_BUBU1",vec![1]),("D_EXACT_THREE",vec![0,1,2]),("E_TWO_UNIQUE",vec![0,1])] {
        session.project=original.clone();
        for(i,key)in keys.iter().enumerate(){if !keep.contains(&i){session.project.database.transaction(|d|d.delete_reference(key))?;}}
        let plan=ps1_export::prepare_project(&source,&target,&mut session,&lib.index,true)?;
        let(_,_,failures)=plan.animation_test_inspection();ensure!(!failures.iter().any(|s|s.starts_with("FAILED")),"Isolation export incomplete: {failures:?}");
        let dir=root.join(name);std::fs::create_dir(&dir)?;
        let path=dir.join(format!("LCK_EXPERIMENTAL_{name}.img"));
        ps1_export::export(plan,&path)?;
        let new=crate::library::Library::open(&path)?;
        let scene=Scene::load(&new.index,"town01")?;let map=scene.field_map_index(&new.index).context("MAP")?;
        ensure!(new.index.entry_bytes(map)?==lib.index.entry_bytes(map)?,"Collision/existing MAP changed");
        let man=scene.field_man_payload(&new.index)?.context("MAN")?;
        let mf=man_section::parse(&man).map_err(anyhow::Error::msg)?;
        let retail_man=Scene::load(&lib.index,"town01")?.field_man_payload(&lib.index)?.context("retail MAN")?;
        let old=man_section::parse(&retail_man).map_err(anyhow::Error::msg)?;
        ensure!(mf.partitions[1].len()==old.partitions[1].len()+keep.len(),"P1 count differs");
        let b=new.index.entry_bytes(map+3)?;let p=new.index.entry_bytes(map+4)?;let(base,_)=reg::native_descriptor_table(&b)?;
        let trace=super::super::native_loader_trace::trace(&b,base,Some(&p))?;
        let origb=lib.index.entry_bytes(map+3)?;let origp=lib.index.entry_bytes(map+4)?;let(obase,_)=reg::native_descriptor_table(&origb)?;
        let origtrace=super::super::native_loader_trace::trace(&origb,obase,Some(&origp))?;
        ensure!(origtrace.models.iter().zip(&trace.models).all(|(a,b)|a.sha256==b.sha256),"Unrelated native models changed");
        let(_,alloc)=super::super::native_loader_trace::tests::execute_original_allocations(&exe,&b,base,Some(&p),true)?;
        let bank=legaia_engine_scene::npc_catalog::scene_anm_bundle(&scene).context("ANM")?;
        let oldbank=legaia_engine_scene::npc_catalog::scene_anm_bundle(&Scene::load(&lib.index,"town01")?).context("retail ANM")?;
        ensure!((0..oldbank.record_count as usize).all(|i|oldbank.record_bytes(i)==bank.record_bytes(i)),"Original animation bytes changed");
        let mut actor_rows=vec![];
        for a in mf.actor_placements(&man).into_iter().filter(|a|a.index>=old.partitions[1].len()) {
            let (_,_,_,mut cpu)=original_actor_installer(&exe,&man,&bank.decoded,a.index,&trace.models[a.model_index as usize].bytes,None)?;
            original_spawn_prerun(&mut cpu,&lib.index.entry_bytes(897)?)?;
            let donor=if keep[actor_rows.len()]==1 {"bubu1"}else{"balden"};let c=resolve(&lib.index,donor,1,None)?;
            ensure!(bank.record_bytes(a.anim_id as usize-1)==c.clip,"Donor ANM source bytes changed");
            actor_rows.push(serde_json::json!({"p1":a.index,"context":old.partitions[0].len()+a.index,"model":a.model_index,"animation":a.anim_id,"clip_sha256":digest(&c.clip),"startup_prerun_completed":true}));
        }
        let sha=digest(&std::fs::read(&path)?);
        let pf=dir.join(format!("{name}.lckproject"));ProjectFile::new(session.project.clone(),pf.clone()).save()?;
        let reopen=ProjectFile::open(&pf,Some(&session.source_fingerprint))?;ensure!(serde_json::to_vec(&reopen.project.database)?==serde_json::to_vec(&session.project.database)?,"Project reopen differs");
        rows.push(serde_json::json!({"case":name,"path":path,"sha256":sha,"active_original_reference_ordinals":keep,"actors":actor_rows,"native_model_count":trace.models.len(),"native_allocations":alloc,"runtime_profile":super::super::native_load_budget::analyze(&b,Some(&p))?,"map_exact_retail":true,"original_model_prefix_preserved":true,"original_anm_records_preserved":true,"gameplay_verified":false}));
    }
    let failedlib=crate::library::Library::open(&failed)?;
    let failedscene=Scene::load(&failedlib.index,"town01")?;
    let dm=failedscene.field_man_payload(&failedlib.index)?.context("failed MAN")?;
    // Before a writer repair, D must reproduce all native scene bytes exactly.
    let dlib=crate::library::Library::open(std::path::Path::new(rows[3]["path"].as_str().unwrap()))?;
    let dscene=Scene::load(&dlib.index,"town01")?;
    let exact=dm==dscene.field_man_payload(&dlib.index)?.context("D MAN")?;
    let report=serde_json::json!({"schema":1,"starting_checkpoint":"34bb3502e37dc7f92a273ee2e2d8910f10cf539f","user_reported_gameplay":"FAILED: original experimental IMG froze loading Rim Elm","failed_image":failed,"failed_sha256":digest(&std::fs::read(&failed)?),"D_man_matches_failed":exact,"cases":rows,"limitations":["Original startup instructions pass with synthetic actor/node allocation, GTE and RNG substitutions","Allocation requests are not live heap capacity proof","No host GUI/emulator launched; all variants await user gameplay testing"]});
    std::fs::write(root.join("ISOLATION_REPORT.json"),serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}
#[test]
#[ignore = "read-only streamed donor class and global/conditional unsupported mechanisms"]
fn stage_b2b_stream_and_unsupported_inventory() -> Result<()> {
    let source = std::path::PathBuf::from(std::env::var_os("LCK_AUDIT_SOURCE").context("retail")?);
    let lib = crate::library::Library::open(&source)?;
    let (mut streamed, mut global, mut zero) = (0, 0, 0);
    for name in ["rikuroa", "town01"] {
        let scene = Scene::load(&lib.index, name)?;
        let man = scene.field_man_payload(&lib.index)?.context("MAN")?;
        let file = man_section::parse(&man).map_err(anyhow::Error::msg)?;
        for p in file.actor_placements(&man) {
            if p.special_model {
                ensure!(
                    resolve(&lib.index, name, p.index, None).is_err(),
                    "Global actor silently substituted"
                );
                global += 1;
                continue;
            }
            if p.anim_id == 0 {
                ensure!(
                    resolve(&lib.index, name, p.index, None).is_err(),
                    "Script-dependent clip guessed"
                );
                zero += 1;
                continue;
            }
            if name == "rikuroa" && streamed == 0 {
                if let Ok(c) = resolve(&lib.index, name, p.index, None) {
                    ensure!(
                        c.origin.anm_source.starts_with("prot:"),
                        "Stream ANM provenance missing"
                    );
                    ensure!(
                        c.pivot.iter().all(|f| f.is_finite()),
                        "Invalid native pose pivot"
                    );
                    streamed += 1;
                    println!(
                        "Streamed donor class: {} P1[{}] {}",
                        name, p.index, c.origin.anm_source
                    );
                }
            }
        }
    }
    ensure!(
        streamed > 0 && global > 0 && zero > 0,
        "Expected streamed and unsupported families not exercised"
    );
    println!(
        "Read-only native class checks: {streamed} streamed closure, {global} rejected global actors, {zero} explicit zero-clip/script-dependent records"
    );
    Ok(())
}
