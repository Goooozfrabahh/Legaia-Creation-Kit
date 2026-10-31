//! Read-only follow-up evidence, not a model-registration or gameplay oracle.
use super::{scene_registration as reg, static_resources};
use crate::app::collision_recipes::digest;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader},
    path::PathBuf,
};

fn resolve(index: &legaia_engine_scene::scene::ProtIndex, id: &str) -> Result<Vec<u8>> {
    let Some((parent, step)) = id.rsplit_once('/') else {
        return Ok(index
            .entry_bytes(
                id.strip_prefix("prot:")
                    .context("Not a PROT identity")?
                    .parse()?,
            )?
            .as_ref()
            .clone());
    };
    let bytes = resolve(index, parent)?;
    if let Some(value) = step.strip_prefix("descriptor:") {
        let (_, i) = value.split_once(':').context("Descriptor identity")?;
        let (base, table) = reg::native_descriptor_table(&bytes)?;
        let d = table
            .used()
            .get(i.parse::<usize>()?)
            .context("Descriptor index")?;
        ensure!(d.size <= 16 * 1024 * 1024, "Bounded decoded source");
        return legaia_lzs::decompress(&bytes[base + d.data_offset as usize..], d.size as usize);
    }
    if let Some(value) = step.strip_prefix("strict-lzs:") {
        return legaia_lzs::decompress_container_strict(&bytes)?
            .get(value.parse::<usize>()?)
            .cloned()
            .context("LZS section");
    }
    if let Some(value) = step.strip_prefix("chunk@") {
        let chunks = legaia_container_formats::parse_streaming(&bytes, 4096)?;
        let c = chunks
            .chunks
            .iter()
            .find(|c| c.header_offset == value.parse::<usize>().unwrap_or(usize::MAX))
            .context("Chunk identity")?;
        return Ok(bytes[c.header_offset + 4..c.header_offset + 4 + c.size as usize].to_vec());
    }
    if let Some(value) = step.strip_prefix("member:") {
        let pack = legaia_container_formats::pack::parse_pack(&bytes)?;
        let m = pack.get(value.parse::<usize>()?).context("Pack member")?;
        return Ok(bytes[m.byte_offset..m.byte_offset + m.size].to_vec());
    }
    if let Some(value) = step.strip_prefix("tmd@") {
        let at = value.parse::<usize>()?;
        let hit = legaia_container_formats::tmd_scan::scan_buffer(&bytes)
            .into_iter()
            .find(|h| h.offset == at)
            .context("Original TMD scan extent")?;
        return Ok(bytes[at..at + hit.byte_len].to_vec());
    }
    if let Some(value) = step.strip_prefix("tim@") {
        let at = value.parse::<usize>()?;
        let tim = legaia_tim::parse_native_upload(&bytes[at..])?;
        return Ok(bytes[at..at + tim.byte_extent()].to_vec());
    }
    anyhow::bail!("Unresolved carrier step {step}")
}

#[test]
#[ignore = "read-only B1.1 exact 45 missing contexts and all 360 raw payloads; new output directory"]
fn stage_b1_1_source_closure_investigation() -> Result<()> {
    let source = PathBuf::from(std::env::var_os("LCK_AUDIT_SOURCE").context("Retail source")?);
    let audit =
        PathBuf::from(std::env::var_os("LCK_BASE_AUDIT").context("Completed original audit")?);
    let output = PathBuf::from(std::env::var_os("LCK_CLOSURE_OUTPUT").context("New output")?);
    ensure!(!output.exists(), "Preserve prior evidence");
    std::fs::create_dir(&output)?;
    let before = digest(&std::fs::read(&source)?);
    let library = crate::library::Library::open(&source)?;
    let mut models = vec![];
    let mut images = vec![];
    let mut known = BTreeSet::new();
    let mut roots = BTreeMap::new();
    for line in BufReader::new(std::fs::File::open(audit.join("asset-nodes.jsonl"))?).lines() {
        let n: Value = serde_json::from_str(&line?)?;
        match n["family"].as_str().unwrap_or("") {
            "NativeModel" => {
                known.insert(n["sha256"].as_str().unwrap_or("").to_owned());
            }
            "TMD" => models.push(n),
            "TIM" => images.push(n),
            "PROT entry" => {
                roots.insert(n["id"].as_str().unwrap().to_owned(), n);
            }
            _ => {}
        }
    }
    let candidates = models
        .into_iter()
        .filter(|n| !known.contains(n["sha256"].as_str().unwrap_or("")))
        .collect::<Vec<_>>();
    ensure!(candidates.len() == 680, "Raw occurrence cohort drift");
    // Hashes below describe geometry extents only, not semantic Base Form IDs.
    // Native pack padding can make a registered member's full hash different
    // from the scanner's exact geometry slab. Retain both coordinate spaces.
    let mut geometry_sources = BTreeMap::<String, Vec<String>>::new();
    for name in library.index.cdname_scene_names() {
        let scene = legaia_engine_scene::scene::Scene::load(&library.index, &name)?;
        let bank = legaia_engine_scene::model_bank::SceneModelBank::build(&scene);
        for (selector, raw) in bank.materialise(&scene).iter().enumerate() {
            if let Some(raw) = raw {
                if let super::model_payload::ModelPayload::BareModel {
                    geometry_sha256, ..
                } = super::model_payload::inspect(raw)
                {
                    geometry_sources
                        .entry(geometry_sha256)
                        .or_default()
                        .push(format!("scene:{name}/model:{selector}"));
                }
            }
        }
    }
    let mut records = vec![];
    let mut unique = BTreeSet::new();
    for n in candidates {
        let id = n["id"].as_str().context("Identity")?;
        let raw = resolve(&library.index, id)?;
        ensure!(
            digest(&raw) == n["sha256"],
            "Raw payload source changed {id}"
        );
        unique.insert(digest(&raw));
        let root = &roots[id.split('/').next().unwrap()];
        let parsed = legaia_tmd::parse(&raw);
        let members=legaia_container_formats::pack::parse_pack(&raw).ok().map(|ms|ms.iter().map(|m|{
            let data=&raw[m.byte_offset..m.byte_offset+m.size];let hit=legaia_container_formats::tmd_scan::scan_buffer(data).into_iter().find(|h|h.offset==0);
            json!({"member":m.index,"offset":m.byte_offset,"size":m.size,"tmd_extent_sha256":hit.as_ref().map(|h|digest(&data[..h.byte_len])),"matches_scene_bank":hit.is_some_and(|h|known.contains(&digest(&data[..h.byte_len])))})
        }).collect::<Vec<_>>());
        let evidence = super::model_payload::inspect(&raw);
        let extent_matches=match &evidence {
            super::model_payload::ModelPayload::BareModel{geometry_sha256,..}=>json!({"geometry_sha256":geometry_sha256,"scene_sources":geometry_sources.get(geometry_sha256)}),
            super::model_payload::ModelPayload::ModelPack{members}=>json!(members.iter().map(|m|json!({"member":m.member,"geometry_sha256":m.geometry_sha256,"scene_sources":geometry_sources.get(&m.geometry_sha256)})).collect::<Vec<_>>()),
            _=>Value::Null,
        };
        records.push(json!({"identity":id,"payload_sha256":digest(&raw),"bytes":raw.len(),"root":root,"geometry_extent_matches":extent_matches,
            "bare_tmd_valid":parsed.is_ok(),"objects":parsed.as_ref().ok().map(|p|p.objects.len()),"pack_members":members,"payload_evidence":evidence,
            "registration_status":"Not an established scene-bank identity; structural extraction is not loader execution",
            "object_window_retention":"Retain source identity; whole packs are containers, not independent placed objects",
            "independently_placeable_verified":false}));
    }
    ensure!(unique.len() == 360, "Raw payload hash cohort drift");
    std::fs::write(
        output.join("RAW_360_SOURCE_READBACK.json"),
        serde_json::to_vec_pretty(
            &json!({"identities":records,"unique_payloads":unique.len(),"occurrences":680}),
        )?,
    )?;
    // Search every inventoried native TIM location, including non-field banks.
    // Coordinate coverage is only a candidate, never permission to import its pixels.
    let failures: Value = serde_json::from_slice(&std::fs::read(
        std::env::var_os("LCK_B1_REMAINING").context("B1 remaining evidence")?,
    )?)?;
    let mut targets = BTreeSet::new();
    for o in failures["objects"].as_array().context("Objects")? {
        for c in o["components"].as_array().unwrap() {
            for span in c["missing_word_spans"].as_array().unwrap() {
                let x = span[0].as_u64().unwrap() as u16;
                let y = span[1].as_u64().unwrap() as u16;
                for xx in x..x + span[2].as_u64().unwrap() as u16 {
                    targets.insert((xx, y));
                }
            }
        }
    }
    let mut matches = vec![];
    let mut errors = vec![];
    let mut seen = BTreeSet::new();
    for n in images {
        let id = n["id"].as_str().unwrap();
        if !seen.insert(n["sha256"].as_str().unwrap_or("").to_owned()) {
            continue;
        }
        let result = (|| -> Result<Value> {
            let raw = resolve(&library.index, id)?;
            ensure!(digest(&raw) == n["sha256"], "TIM provenance changed");
            let tim = legaia_tim::parse_native_upload(&raw)?;
            let mut covered = vec![];
            for &(x, y) in &targets {
                let image = x >= tim.image.fb_x
                    && x < tim.image.fb_x + tim.image.fb_w
                    && y >= tim.image.fb_y
                    && y < tim.image.fb_y + tim.image.h;
                let clut = tim.clut.as_ref().is_some_and(|c| {
                    y == c.fb_y && x >= c.fb_x && (x as usize) < c.fb_x as usize + c.entries.len()
                });
                if image || clut {
                    covered.push(json!({"word":[x,y],"image":image,"palette":clut}));
                }
            }
            Ok(
                json!({"identity":id,"sha256":n["sha256"],"covered_words":covered,"loader_relationship_proven":false}),
            )
        })();
        match result {
            Ok(v) => {
                if !v["covered_words"].as_array().unwrap().is_empty() {
                    matches.push(v);
                }
            }
            Err(e) => errors.push(json!({"identity":id,"error":format!("{e:#}")})),
        }
    }
    let character_bytes = library
        .index
        .entry_bytes(legaia_actor_formats::field_char_textures::PROT_ENTRY_INDEX)?;
    let chars = legaia_actor_formats::field_char_textures::parse(&character_bytes)?;
    let mut character_vram = legaia_tim::Vram::new();
    chars.upload_to_vram(&mut character_vram, false);
    let character_candidate_words = targets
        .iter()
        .filter(|&&(x, y)| character_vram.is_written(x as usize, y as usize))
        .copied()
        .collect::<Vec<_>>();
    let mut contexts = vec![];
    for o in failures["objects"].as_array().unwrap() {
        let scene = legaia_engine_scene::scene::Scene::load(
            &library.index,
            o["source_scene"].as_str().unwrap(),
        )?;
        let c = static_resources::resolve_context(&library.index, &scene)?;
        let selector = o["selector"].as_u64().unwrap() as u16;
        let raw = c.models[selector as usize].as_ref().context("Model")?;
        let d = c.dependencies(&scene, selector, raw)?;
        ensure!(
            d.tmd_sha256 == o["source_tmd_sha256"],
            "Model identity drift"
        );
        contexts.push(json!({"identity":o["identity"],"dependencies":d}));
    }
    ensure!(
        before == digest(&std::fs::read(&source)?),
        "Protected retail changed"
    );
    std::fs::write(
        output.join("MISSING_WORD_RETAIL_SEARCH.json"),
        serde_json::to_vec_pretty(
            &json!({"word_targets":targets,"TIM_payloads_searched":seen.len(),"coordinate_candidates":matches,"unresolved_carriers":errors,"contexts":contexts,"complete_character_resident_TIM_count":chars.textures.len(),"character_candidate_words":character_candidate_words,"source_sha256":before,"source_unchanged":true,"runtime_producer_verified":false,"coordinate_matches_are_not_dependencies":true}),
        )?,
    )?;
    println!(
        "B1.1: 360 hashes/680 occurrences read back; {} native TIM payload identities searched",
        seen.len()
    );
    Ok(())
}

#[test]
#[ignore = "read-only native audio/animation consumer checks for all unresolved raw tags"]
fn stage_b1_1_non_geometry_consumer_evidence() -> Result<()> {
    let source = PathBuf::from(std::env::var_os("LCK_AUDIT_SOURCE").context("Retail")?);
    let output = PathBuf::from(
        std::env::var_os("LCK_CLOSURE_OUTPUT").context("Completed investigation directory")?,
    );
    let report = output.join("NON_GEOMETRY_CONSUMERS.json");
    ensure!(!report.exists(), "New evidence required");
    let all: Value =
        serde_json::from_slice(&std::fs::read(output.join("RAW_360_SOURCE_READBACK.json"))?)?;
    let library = crate::library::Library::open(&source)?;
    let mut seen = BTreeSet::new();
    let mut evidence = vec![];
    for r in all["identities"].as_array().context("Identities")? {
        if r["payload_evidence"]["kind"] != "Unresolved"
            || !seen.insert(r["payload_sha256"].as_str().unwrap().to_owned())
        {
            continue;
        }
        let id = r["identity"].as_str().unwrap();
        let raw = resolve(&library.index, id)?;
        ensure!(digest(&raw) == r["payload_sha256"], "Source changed");
        let entry = r["root"]["fields"]["index"]
            .as_u64()
            .context("Root entry")? as u32;
        let root = library.index.entry_bytes(entry)?;
        let audio = legaia_container_formats::scene_vab_stream::detect(&root);
        let animation = legaia_actor_formats::player_anm::parse(&raw);
        let consumer = if animation.is_ok() {
            "Native ANM-format animation data"
        } else if audio.is_some() {
            "Component of native VAB-prefixed audio carrier; not validated TMD"
        } else {
            "Native consumer unresolved; not validated TMD"
        };
        evidence.push(json!({"identity":id,"sha256":r["payload_sha256"],"native_consumer_evidence":consumer,"audio_carrier_detected":audio.is_some(),"ANM_payload_parsed":animation.is_ok(),"placeability_verified":false}));
    }
    ensure!(seen.len() == 80, "Unresolved raw-tag cohort drift");
    std::fs::write(report, serde_json::to_vec_pretty(&evidence)?)?;
    Ok(())
}
