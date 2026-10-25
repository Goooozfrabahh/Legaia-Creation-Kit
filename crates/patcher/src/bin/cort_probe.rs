//! Read-only diagnostics for Legend of Legaia's original teenage Cort field model.
//! Never writes to the source disc, patched disc, or memory card.
//! Copy to crates/patcher/src/bin/cort_probe.rs and run:
//! cargo +stable-x86_64-pc-windows-gnu run --release -p legaia-patcher --bin cort_probe

use std::fs;
use anyhow::{Context, Result};
use legaia_disc_patch::disc::DiscPatcher;
use legaia_actor_formats::player_anm;
use legaia_scene_formats::{field_objects::FIELD_ACTOR_PACK_BIAS, scene_asset_table};

const ORIGINAL: &str = r"D:\Emulators\Games\Legend of Legaia\LegendOfLegaia.img";
const CORT_SCENE_MODEL_INDEX: usize = 159;
const CORT_ANIM_ID: usize = 31;
const CONC2_EXPECTED_PROT_ENTRY: usize = 624;

fn main() -> Result<()> {
    let bytes = fs::read(ORIGINAL).with_context(|| format!("read {ORIGINAL}"))?;
    let mut disc = DiscPatcher::open(bytes).context("open original PS1 disc image")?;
    let bias = FIELD_ACTOR_PACK_BIAS as usize;
    let pack_index = CORT_SCENE_MODEL_INDEX.checked_sub(bias)
        .context("model index is smaller than the field party-pool prefix")?;

    println!("Cort source: conc2; runtime model index {CORT_SCENE_MODEL_INDEX}; ANM id {CORT_ANIM_ID}");
    println!("Runtime pool prefix {bias}; corresponding scene-mesh-pool index {pack_index}");
    println!("Checking expected conc2 entry, then neighboring scene-block entries (read-only)...\n");

    let mut found = false;
    let mut candidates = vec![CONC2_EXPECTED_PROT_ENTRY];
    for idx in 620..=635 {
        if idx != CONC2_EXPECTED_PROT_ENTRY {
            candidates.push(idx);
        }
    }
    for idx in candidates {
        let Ok(entry) = disc.read_entry_footprint(idx) else { continue };
        let pool = scene_asset_table::mesh_pool(&entry);
        if pool.len() <= pack_index { continue; }
        let mesh = &pool[pack_index];
        let Ok(tmd) = legaia_tmd::parse(&mesh.bytes) else { continue; };
        found = true;
        println!("PROT[{idx}] candidate: {} scene-pool meshes", pool.len());
        println!("  model #159 = pool[{pack_index}]: descriptor slot {}, pack member {}", mesh.slot, mesh.member);
        println!("  TMD size = {} bytes, object/bone parts = {}", mesh.bytes.len(), tmd.objects.len());
        if tmd.objects.len() == 10 {
            println!("  RESULT: 10-part geometry: promising for Vahn's 10-part field rig");
        } else {
            println!("  RESULT: not a 10-part geometry rig; conversion will require special handling");
        }

        // Scene actor clip numbers are one-based. The scene's animation asset
        // is counterintuitively tagged type 0x05 (not 0x06).
        match scene_asset_table::decode_slot_by_type(&entry, player_anm::SCENE_ANM_TYPE_BYTE) {
            Some(Ok(data)) => match player_anm::parse(&data) {
                Ok(bundle) => match bundle.record(CORT_ANIM_ID - 1) {
                    Ok(rec) => {
                        println!("  scene ANM record 30: bones={}, frames={}, flags={:#06x}",
                            rec.bone_count, rec.frame_count, rec.flag);
                        println!("  mesh/animation part counts match: {}", rec.bone_count as usize == tmd.objects.len());
                    }
                    Err(e) => println!("  scene ANM record 30 not accessible: {e}"),
                },
                Err(e) => println!("  ANM data was present but could not parse: {e}"),
            },
            Some(Err(e)) => println!("  ANM type-5 slot decode error: {e}"),
            None => println!("  No type-5 ANM slot in this entry (it may live in another scene entry)"),
        }
        println!();
    }
    if !found {
        println!("No valid model #159 in the expected nearby entries. Do not guess a PROT offset.");
        println!("We'll trace conc2's actual scene mesh sources before editing any asset.");
    }
    println!("READ-ONLY: No game image or memory card was modified.");
    Ok(())
}
