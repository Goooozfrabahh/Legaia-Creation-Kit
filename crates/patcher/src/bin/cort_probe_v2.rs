//! Read-only Cort 159 source verification for Legend of Legaia (PS1).
//! Fixes cort_probe.rs's incorrect subtraction of the five global party meshes.
//! Place in crates/patcher/src/bin/cort_probe_v2.rs.
//! Run: cargo +stable-x86_64-pc-windows-gnu run --release -p legaia-patcher --bin cort_probe_v2

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
    let image = fs::read(ORIGINAL).with_context(|| format!("read {ORIGINAL}"))?;
    let disc = DiscPatcher::open(image).context("open original PS1 image")?;
    let party_prefix = FIELD_ACTOR_PACK_BIAS as usize;

    // Model IDs below 0xF0 index the SCENE bank directly; the five party
    // TMDs are a prefix in the *global runtime pointer table*, NOT in the
    // scene pack. 159 therefore means scene pack member 159, not member 154.
    let pack_index = CORT_SCENE_MODEL_INDEX;
    println!("Cort: conc2 model_id={CORT_SCENE_MODEL_INDEX}, anim_id={CORT_ANIM_ID}");
    println!("CORRECT mapping: scene pack member {pack_index}; global runtime pool slot {}", party_prefix + pack_index);
    println!("Old probe used member 154, which was not Cort.\n");

    let mut candidates = vec![CONC2_EXPECTED_PROT_ENTRY];
    for idx in 620..=635 {
        if idx != CONC2_EXPECTED_PROT_ENTRY { candidates.push(idx); }
    }
    let mut hit = false;
    for idx in candidates {
        let Ok(entry) = disc.read_entry_footprint(idx) else { continue };
        let pool = scene_asset_table::mesh_pool(&entry);
        if pool.len() <= pack_index { continue; }
        let mesh = &pool[pack_index];
        let Ok(tmd) = legaia_tmd::parse(&mesh.bytes) else { continue; };
        println!("PROT[{idx}]: scene pool count={}; member[159] slot={} member={} size={} bytes nobj={}",
            pool.len(), mesh.slot, mesh.member, mesh.bytes.len(), tmd.objects.len());
        for neighbor in 157..=161 {
            if let Some(m) = pool.get(neighbor) {
                if let Ok(t) = legaia_tmd::parse(&m.bytes) {
                    println!("  nearby pool[{neighbor}]: {} objects; {} bytes", t.objects.len(), m.bytes.len());
                }
            }
        }
        match scene_asset_table::decode_slot_by_type(&entry, player_anm::SCENE_ANM_TYPE_BYTE) {
            Some(Ok(data)) => match player_anm::parse(&data) {
                Ok(bundle) => match bundle.record(CORT_ANIM_ID - 1) {
                    Ok(clip) => {
                        println!("  ANM record[30]: {} bones, {} frames, flags {:#06x}",
                            clip.bone_count, clip.frame_count, clip.flag);
                        println!("  Model and clip part counts match: {}", tmd.objects.len() == clip.bone_count as usize);
                    }
                    Err(e) => println!("  ANM[30] missing: {e}"),
                },
                Err(e) => println!("  ANM parse error: {e}"),
            },
            Some(Err(e)) => println!("  ANM decompression error: {e}"),
            None => println!("  This PROT entry has no ANM bundle"),
        }
        println!();
        if idx == CONC2_EXPECTED_PROT_ENTRY { hit = true; break; }
    }
    if !hit { println!("Did not confirm PROT[624]; need to resolve the scene model bank across its full CDNAME block."); }
    println!("READ ONLY: no modified image, original image, or memory card was changed.");
    Ok(())
}
