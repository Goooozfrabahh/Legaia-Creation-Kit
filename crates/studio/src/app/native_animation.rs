//! Detached native field-animation compiler. No donor story/dialogue bytecode.
use super::{
    collision_recipes::digest,
    form_database::{FormKey, Transform},
    native_asset_plan::{self as assets, scene_registration as reg},
};
use anyhow::{Context, Result, ensure};
use legaia_actor_formats::player_anm;
use legaia_engine_scene::scene::{ProtIndex, Scene};
use legaia_scene_formats::{man_section, scene_asset_table as table};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(super) struct State {
    pub clip: u8,
    pub frame: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Origin {
    pub scene: String,
    pub actor: usize,
    pub man_sha256: String,
    pub model_selector: u8,
    pub model_sha256: String,
    pub model_source: String,
    pub anm_source: String,
    pub bank_sha256: String,
    pub carrier_sha256: String,
    pub bank_address: serde_json::Value,
    pub record: usize,
    pub record_offset: u32,
    pub clip_sha256: String,
}
pub(super) struct Closure {
    pub origin: Origin,
    pub model: Vec<u8>,
    pub clip: Vec<u8>,
    pub pivot: [f32; 3],
    pub context: assets::static_resources::ContextData,
    pub behavior: super::behavior_resources::Dependencies,
}
/// Resolve the actual last native MAN/ANM writers in descriptor -> PAC order.
fn active_actor_payloads(bundle:&[u8],pac:Option<&[u8]>)->Result<BTreeMap<u8,(bool,Vec<u8>)>> {
    let(base,header)=reg::native_descriptor_table(bundle)?;let mut result=BTreeMap::new();
    let trace=super::native_loader_trace::trace(bundle,base,pac)?;
    ensure!(trace.gaps.is_empty(),"Native actor dependency loader profile unresolved: {:?}",trace.gaps);
    for d in header.used().iter().filter(|d|matches!(d.type_byte,3|5)){
        result.insert(d.type_byte,(true,legaia_lzs::decompress(&bundle[base+d.data_offset as usize..],d.size as usize)?));
    }
    if reg::native_stream_mode(bundle)==Some(0x14){let bytes=pac.context("Native actor PAC missing")?;let mut at=0;
        loop {let word=u32::from_le_bytes(bytes.get(at..at+4).context("Native actor stream terminator missing")?.try_into()?);let size=(word&0xffffff)as usize;if size==0{break;}
            let body=bytes.get(at+4..at+4+size).context("Native actor stream payload extent")?;let kind=(word>>24)as u8;
            if matches!(kind,3|5){result.insert(kind,(false,body.to_vec()));}at+=4+(size&!3);
        }
    }
    ensure!(result.contains_key(&3)&&result.contains_key(&5),"Active native MAN/ANM writer missing");Ok(result)
}
fn reject_other_actor_carriers(scene:&Scene,map:usize)->Result<()> {
    for entry in &scene.entries {
        if entry.idx as usize==map+3||entry.idx as usize==map+4||entry.class!=legaia_disc_audit::categorize::Class::DataFieldStreaming{continue;}
        let stream=legaia_container_formats::parse_streaming(&entry.bytes,4096)?;
        ensure!(!stream.chunks.iter().any(|c|matches!(c.type_byte,3|5)),"Additional actor dependency carrier requires an explicit native load-order adapter");
    }Ok(())
}
/// Local scene-bank skeletal ANM. Special/equipment/global banks remain explicit failures.
pub(super) fn resolve(
    index: &ProtIndex,
    name: &str,
    actor: usize,
    state: Option<&State>,
) -> Result<Closure> {
    let scene = Scene::load(index, name)?;
    let man = scene
        .field_man_payload(index)?
        .context("Donor MAN unavailable")?;
    let mf = man_section::parse(&man).map_err(anyhow::Error::msg)?;
    let placement = mf
        .actor_placement(&man, actor)
        .context("Donor actor identity unavailable")?;
    ensure!(actor > 0, "MAN controller is not an actor prototype");
    ensure!(
        !placement.special_model,
        "Global/party model-bank and equipment adapter unresolved; no scene-bank substitution"
    );
    let state = state.cloned().unwrap_or(State {
        clip: placement.anim_id,
        frame: 0,
    });
    ensure!(
        state.frame == 0,
        "Native initial frame other than zero requires clip-driver state relocation"
    );
    ensure!(
        state.clip > 0,
        "No explicit initial ANM clip; donor script is not copied to guess one"
    );
    let context = assets::static_resources::resolve_actor_context(index, &scene)?;
    let model = context
        .models
        .get(placement.model_index as usize)
        .and_then(|m| m.clone())
        .context("Native actor model selector unresolved")?;
    let parsed = legaia_tmd::parse(&model)?;
    let bank = legaia_engine_scene::npc_catalog::scene_anm_bundle(&scene)
        .context("Donor ANM bank unavailable")?;
    let prot = legaia_engine_scene::npc_catalog::scene_anm_prot(&scene)
        .context("ANM carrier identity unavailable")?;
    let carrier = scene
        .entries
        .iter()
        .find(|e| e.idx == prot)
        .context("ANM original carrier missing")?;
    let mut addresses = vec![];
    if let Ok((base, table)) = reg::native_descriptor_table(&carrier.bytes) {
        for (slot, d) in table
            .used()
            .iter()
            .enumerate()
            .filter(|(_, d)| d.type_byte == 5)
        {
            if legaia_lzs::decompress(
                &carrier.bytes[base + d.data_offset as usize..],
                d.size as usize,
            )
            .ok()
            .as_deref()
                == Some(bank.decoded.as_slice())
            {
                addresses.push(serde_json::json!({"kind":"descriptor","table_base":base,"descriptor":slot,"stored_offset":base+d.data_offset as usize,"decoded_size":d.size}));
            }
        }
    }
    if addresses.is_empty() {
        if let Ok(stream) = legaia_container_formats::parse_streaming(&carrier.bytes, 4096) {
            for chunk in stream.chunks.iter().filter(|c| c.type_byte == 5) {
                if carrier
                    .bytes
                    .get(chunk.header_offset + 4..chunk.header_offset + 4 + chunk.size as usize)
                    == Some(bank.decoded.as_slice())
                {
                    addresses.push(serde_json::json!({"kind":"stream","header_offset":chunk.header_offset,"stored_offset":chunk.header_offset+4,"decoded_size":chunk.size}));
                }
            }
        }
    }
    ensure!(
        addresses.len() == 1,
        "ANM bank exact descriptor/stream source is ambiguous or unresolved"
    );
    let source_map=scene.field_map_index(index).context("Actor donor native field context missing")? as usize;
    reject_other_actor_carriers(&scene,source_map)?;
    let source_bundle=index.entry_bytes(source_map as u32+3)?;
    let source_pac=reg::native_stream_mode(&source_bundle).map(|_|index.entry_bytes(source_map as u32+4)).transpose()?;
    let source_pac_bytes=source_pac.as_deref().map(|p|p.as_slice());
    let active=active_actor_payloads(&source_bundle,source_pac_bytes)?;
    ensure!(active[&3].1==man&&active[&5].1==bank.decoded,"Discovered MAN/ANM source is not the actual active native loader dependency");
    let(base,_)=reg::native_descriptor_table(&source_bundle)?;
    let native_models=super::native_loader_trace::trace(&source_bundle,base,source_pac_bytes)?;
    ensure!(native_models.models.len()==context.models.len()&&native_models.models.get(placement.model_index as usize).is_some_and(|m|m.sha256==digest(&model)),"Donor actor model selector differs from complete native registration walk");
    let record = state.clip as usize - 1;
    let header = bank.record(record)?;
    ensure!(
        header.marker_1 == 0x080c && header.flag != 0,
        "Unknown ANM marker/rate format"
    );
    ensure!(
        header.frame_count > 0 && header.bone_count as usize == parsed.objects.len(),
        "Native ANM bone count differs from TMD object table; pose/equipment adapter required"
    );
    let clip = bank.record_bytes(record).to_vec();
    let mut behavior =
        super::behavior_resources::Dependencies::from_man(&format!("{name}:MAN"), &man, actor)?;
    behavior.bind_scene_animation(&format!("{name}:MAN"), &man, actor, &scene)?;
    let origin = Origin {
        scene: name.into(),
        actor,
        man_sha256: digest(&man),
        model_selector: placement.model_index,
        model_sha256: digest(&model),
        model_source: format!("{:?}",context.registrations.get(placement.model_index as usize).context("Native model source identity missing")?),
        anm_source: format!("prot:{prot}:decoded-type-5"),
        bank_sha256: digest(&bank.decoded),
        carrier_sha256: digest(&carrier.bytes),
        bank_address: addresses.remove(0),
        record,
        record_offset: bank.record_offsets[record],
        clip_sha256: digest(&clip),
    };
    let mut mesh = crate::raster::Model::from_tmd(&model)
        .context("Animated model preview format unsupported")?;
    if let Some(base_record)=placement.anim_id.checked_sub(1) {
        let base_header=bank.record(base_record as usize)?;
        ensure!(base_header.bone_count as usize==parsed.objects.len(),"Base pose/object binding requires another native adapter");
        let animation=bank.record_to_monster_animation(base_record as usize).context("Base ANM pose sampling unsupported")?;
        mesh.pose(animation.frames.first().context("Base ANM has no initial pose")?);
    }

    ensure!(!mesh.positions.is_empty(), "Empty animated model");
    let pivot = [0, 1, 2].map(|axis| {
        let lo = mesh
            .positions
            .iter()
            .map(|p| p[axis])
            .fold(f32::INFINITY, f32::min);
        let hi = mesh
            .positions
            .iter()
            .map(|p| p[axis])
            .fold(f32::NEG_INFINITY, f32::max);
        (lo + hi) * 0.5 * if axis == 1 { -1. } else { 1. }
    });
    Ok(Closure {
        origin,
        model,
        clip,
        pivot,
        context,
        behavior,
    })
}
/// Rebuild offset table without changing any original record bytes or IDs.
pub(super) fn append_clips(original: &[u8], clips: &[Vec<u8>]) -> Result<(Vec<u8>, Vec<u8>)> {
    let bank = player_anm::parse(original)?;
    let mut records = (0..bank.record_count as usize)
        .map(|i| bank.record_bytes(i).to_vec())
        .collect::<Vec<_>>();
    let mut ids = vec![];
    for raw in clips {
        let i = if let Some(i) = records.iter().position(|r| r == raw) {
            i
        } else {
            let i = records.len();
            records.push(raw.clone());
            i
        };
        ensure!(i < 255, "Native animation selector byte exhausted");
        ids.push((i + 1) as u8);
    }
    let mut output = vec![0; 4 + 4 * records.len()];
    output[..4].copy_from_slice(&(records.len() as u32).to_le_bytes());
    for (i, record) in records.iter().enumerate() {
        let offset = u32::try_from(output.len())?;
        output[4 + 4 * i..8 + 4 * i].copy_from_slice(&offset.to_le_bytes());
        output.extend(record);
    }
    let read = player_anm::parse(&output)?;
    for (i, record) in records.iter().enumerate() {
        let header = read.record(i)?;
        ensure!(
            header.marker_1 == 0x080c
                && header.bone_count > 0
                && header.frame_count > 0
                && header.flag != 0,
            "Unsupported ANM record format in merged bank"
        );
        ensure!(
            read.record_bytes(i) == record,
            "ANM exact record readback failed"
        );
    }
    Ok((output, ids))
}
fn tile(v: f32) -> Result<u8> {
    ensure!(v.is_finite(), "Nonfinite actor coordinate");
    (0..=255u16)
        .find(|b| {
            let b = *b as u8;
            let n = (b & 127) as i32 * 128 + if b & 128 != 0 { 128 } else { 64 };
            (v - n as f32).abs() < 0.01
        })
        .map(|b| b as u8)
        .context("Actor X/Z must be an exact native half-tile or tile center; no silent snapping")
}
fn fixed(v: f32, scale: f32) -> Result<i16> {
    ensure!(v.is_finite(), "Nonfinite native transform");
    let n = v * scale;
    ensure!(
        (n - n.round()).abs() < 0.01,
        "Transform not exactly representable by native fixed point"
    );
    Ok(i16::try_from(n.round() as i32)?)
}
/// Native spawn prologue, authored for the new context only. Never donor behavior.
fn actor_record(model: u8, clip: u8, t: &Transform) -> Result<Vec<u8>> {
    ensure!(
        t.rotation[0] == 0. && t.rotation[2] == 0.,
        "Native pitch/roll composition with editor pose pivot remains unverified"
    );
    ensure!(
        model < 0xf0 && clip > 0,
        "Native actor selector aliases a party/global bank"
    );
    ensure!(
        t.scale[0] > 0. && t.scale.iter().all(|s| (*s - t.scale[0]).abs() < 0.00001),
        "Native uniform render scale required; anisotropic animation transform unsupported"
    );
    let x = tile(t.position[0])?;
    let z = tile(t.position[2])?;
    let mut out = vec![0, model, clip, x, z, 0x24, 0x22, clip];
    // Native immediate slot writes, ticks=0. Header animation is independently bound.
    for (sub, value) in [
        (0, fixed(t.scale[0], 4096.)?),
        (3, fixed(t.rotation[0], 4096. / std::f32::consts::TAU)?),
        (4, fixed(t.rotation[2], 4096. / std::f32::consts::TAU)?),
        (8, fixed(t.rotation[1], 4096. / std::f32::consts::TAU)?),
    ] {
        out.extend([0x4c, 0x40 | sub]);
        out.extend(value.to_le_bytes());
        out.extend([0, 0]);
    }
    // Fixed absolute Y: actor flag bit29 enables the native inverted-Y mirror.
    out.extend([0x31, 29, 0x4c, 0x42]);
    out.extend(fixed(t.position[1], 1.)?.to_le_bytes());
    out.extend([0, 0, 0x21, 0x21]);
    Ok(out)
}
#[derive(Clone, Debug, Serialize)]
pub(super) struct Seat {
    pub origin: Option<Origin>,
    pub registered_model_sha256: Option<String>,
    pub reference: FormKey,
    pub base: FormKey,
    pub p1: usize,
    pub context: usize,
    pub model: u8,
    pub animation: u8,
    pub transform: Transform,
}
pub(super) fn append_actors(
    man: &[u8],
    requests: &[(FormKey, FormKey, u8, u8, Transform)],
) -> Result<(Vec<u8>, Vec<Seat>)> {
    ensure!(!requests.is_empty(), "No animated reference requests");
    let mf = man_section::parse(man).map_err(anyhow::Error::msg)?;
    let n0 = mf.partitions[0].len();
    let n1 = mf.partitions[1].len();
    let n2 = mf.partitions[2].len();
    let n = requests.len();
    ensure!(
        n1 > 0 && n1 + n <= i16::MAX as usize && n0 + n1 + n2 + n < 0xf8,
        "Native context namespace aliases reserved player/system IDs"
    );
    let mut keys = BTreeSet::new();
    let mut records = vec![];
    let mut seats = vec![];
    for (i, (reference, base, model, clip, t)) in requests.iter().enumerate() {
        ensure!(
            keys.insert(reference),
            "Duplicate animated Reference identity"
        );
        records.push(actor_record(*model, *clip, t)?);
        seats.push(Seat {
            origin: None,
            registered_model_sha256: None,
            reference: reference.clone(),
            base: base.clone(),
            p1: n1 + i,
            context: n0 + n1 + i,
            model: *model,
            animation: *clip,
            transform: *t,
        });
    }
    let section = mf.sections[0].offset;
    ensure!(
        mf.sections.iter().all(|s| s.offset >= section),
        "MAN sections do not follow record region"
    );
    ensure!(
        mf.partitions
            .iter()
            .flatten()
            .all(|r| mf.data_region_offset + (*r as usize) < section),
        "MAN contains records after section0; writer class unsupported"
    );
    let insert = man_section::RECORDS_BEGIN_OFFSET + 3 * (n0 + n1);
    let relative = mf.header.u24_at_28 as usize;
    let mut output = man[..insert].to_vec();
    let mut rel = relative;
    for record in &records {
        ensure!(rel < 0x1000000, "MAN u24 offset exhausted");
        output.extend_from_slice(&(rel as u32).to_le_bytes()[..3]);
        rel += record.len();
    }
    output.extend_from_slice(&man[insert..section]);
    for r in &records {
        output.extend(r);
    }
    output.extend_from_slice(&man[section..]);
    ensure!(rel < 0x1000000, "MAN section offset exhausted");
    output[0x24..0x26].copy_from_slice(&((n1 + n) as i16).to_le_bytes());
    output[0x28..0x2b].copy_from_slice(&(rel as u32).to_le_bytes()[..3]);
    // Keep old P2 context identity semantics when the appended P1 band shifts it.
    let remap = (n0 + n1..n0 + n1 + n2)
        .map(|id| (id as u8, (id + n) as u8))
        .collect::<BTreeMap<_, _>>();
    if !remap.is_empty() {
        let refs = super::native_script_relocation::references(&output)?;
        if refs.iter().any(|r| remap.contains_key(&r.target)) {
            output = super::native_script_relocation::compile(&output, &remap)?.apply(&output)?;
        }
        let read = man_section::parse(&output).map_err(anyhow::Error::msg)?;
        for motion in legaia_scene_formats::man_motion::motion_records(&output, &read) {
            for (i, b) in motion.bindings.iter().enumerate() {
                if let Some(to) = remap.get(&b.actor_id) {
                    output[motion.offset + 3 + 2 * i] = *to;
                }
            }
        }
    }
    let read = man_section::parse(&output).map_err(anyhow::Error::msg)?;
    for s in &seats {
        let a = read
            .actor_placement(&output, s.p1)
            .context("New native actor readback missing")?;
        ensure!(
            a.model_index == s.model && a.anim_id == s.animation,
            "Native actor registration readback differs"
        );
    }
    // All original records are preserved modulo the explicit context remap above.
    ensure!(
        read.partitions[0].len() == n0
            && read.partitions[1].len() == n1 + n
            && read.partitions[2].len() == n2,
        "Native partition identities changed"
    );
    Ok((output, seats))
}
/// Rewrite only decoded MAN/ANM slots; stored unrelated payloads are exact copies.
pub(super) fn rewrite_bundle(
    source: &[u8],
    replacements: &BTreeMap<u8, Vec<u8>>,
    max_encoded: usize,
) -> Result<Vec<u8>> {
    let (base, header) = reg::native_descriptor_table(source)?;
    let mut output = source[..base + 8 + header.count * 8].to_vec();
    let mut total = 0u32;
    let mut found = BTreeSet::new();
    for (i, d) in header.used().iter().enumerate() {
        let start = base + d.data_offset as usize;
        let end = header
            .used()
            .get(i + 1)
            .map_or(source.len(), |d| base + d.data_offset as usize);
        ensure!(
            start < end && end <= source.len(),
            "Invalid native descriptor extent"
        );
        let off = (output.len() - base) as u32;
        let at = base + 8 + i * 8;
        let size = if let Some(raw) = replacements.get(&d.type_byte) {
            ensure!(
                found.insert(d.type_byte),
                "Multiple native singleton dependency slots"
            );
            ensure!(
                raw.len() <= 0xffffff,
                "Native dependency decoded width exceeded"
            );
            let (_, used) = legaia_lzs::decompress_tracked(&source[start..end], d.size as usize)?;
            output.extend(
                legaia_disc_patch::compress_within(raw, max_encoded)
                    .context("Native dependency compression budget exceeded")?,
            );
            // Keep nonzero opaque member suffix; discard only zero padding.
            let suffix = &source[start + used..end];
            if suffix.iter().any(|v| *v != 0) {
                output.extend(suffix);
            }
            raw.len() as u32
        } else {
            output.extend_from_slice(&source[start..end]);
            d.size
        };
        total = total
            .checked_add(size)
            .context("Native decoded total overflow")?;
        output[at..at + 4]
            .copy_from_slice(&table::encode_size_word(d.type_byte, size).to_le_bytes());
        output[at + 4..at + 8].copy_from_slice(&off.to_le_bytes());
    }
    ensure!(
        found.len() == replacements.len(),
        "Destination lacks a singleton MAN/type-5 descriptor; streaming dependency writer required"
    );
    output[base + 4..base + 8].copy_from_slice(&total.to_le_bytes());
    ensure!(
        output.len().div_ceil(2048) * 2048 <= max_encoded.min(404480),
        "Native encoded asset arena exhausted"
    );
    let (b, h) = reg::native_descriptor_table(&output)?;
    for (kind, expected) in replacements {
        let d = h
            .used()
            .iter()
            .find(|d| d.type_byte == *kind)
            .context("Rewritten dependency missing")?;
        ensure!(
            legaia_lzs::decompress(&output[b + d.data_offset as usize..], d.size as usize)?
                == *expected,
            "Native dependency bytes differ after compression"
        );
    }
    Ok(output)
}

#[derive(Serialize)]
pub(super) struct Compilation {
    pub resource_dependencies: Vec<serde_json::Value>,
    pub seats: Vec<Seat>,
    pub donors: Vec<Origin>,
    pub encoded_bundle_bytes: usize,
    pub added_animation_bytes: usize,
    pub runtime_profile: serde_json::Value,
    pub reference_transforms: BTreeMap<FormKey, Transform>,
    pub behavior_preserving: bool,
    pub gameplay_verified: bool,
    pub limitations: Vec<String>,
}
/// Compile a complete desired reference set against the mounted master, not a
/// previous output. Omitted/deleted references own no allocations in the new output.
pub(super) fn compile_project(
    mut disc: legaia_disc_patch::disc::DiscPatcher,
    session: &super::form_bridge::Session,
    index: &ProtIndex,
) -> Result<(legaia_disc_patch::disc::DiscPatcher, Compilation)> {
    use assets::{
        NativeAssetPlan, Origin as ResourceOrigin, ResourceKind,
        texture_relocation::{Planner, field_runtime_reservations, multi_tim::UploadTrace},
    };
    let cell = session
        .cell
        .as_ref()
        .context("Animated destination Cell missing")?;
    let refs = session.project.database.references();
    let mut requests = refs
        .values()
        .filter(|r| {
            &r.cell == cell
                && !r.deleted
                && !r.disabled
                && r.flags.contains("native_animated_reference")
        })
        .cloned()
        .collect::<Vec<_>>();
    requests.sort_by_key(|r| r.key.clone());
    if requests.is_empty() {
        return Ok((
            disc,
            Compilation {
                resource_dependencies: vec![],
                seats: vec![],
                donors: vec![],
                encoded_bundle_bytes: 0,
                added_animation_bytes: 0,
                runtime_profile: serde_json::Value::Null,
                reference_transforms: BTreeMap::new(),
                behavior_preserving: false,
                gameplay_verified: false,
                limitations: vec![
                    "No active animated References; mounted-master resources unchanged".into(),
                ],
            },
        ));
    }
    // All builds begin at the exact mounted master; inherited ownership is not guessed.
    let source = std::fs::read(&session.source)?;
    ensure!(
        digest(&source) == session.project.master.disc_sha256,
        "Animated source differs from project master; inherited resource ownership adapter required"
    );
    let target_name = session
        .project
        .cells
        .iter()
        .find(|c| &c.key == cell)
        .context("Destination Cell adapter missing")?
        .source_scene
        .clone();
    let target = Scene::load(index, &target_name)?;
    let map = target
        .field_map_index(index)
        .context("Destination native MAP identity missing")? as usize;
    let limits = reg::Limits {
        max_models: 195,
        max_decoded: 16 * 1024 * 1024,
        max_encoded: 404480,
    };
    let mut bundle = disc.read_entry_footprint(map + 3)?;
    let (base, header) = reg::native_descriptor_table(&bundle)?;
    let decode = |kind| -> Result<Vec<u8>> {
        let mut slots = header.used().iter().filter(|d| d.type_byte == kind);
        let d = slots
            .next()
            .context("Destination singleton dependency descriptor unavailable")?;
        ensure!(
            slots.next().is_none(),
            "Ambiguous destination dependency bank"
        );
        legaia_lzs::decompress(&bundle[base + d.data_offset as usize..], d.size as usize)
    };
    let man = decode(3)?;
    let anm = decode(5)?;
    let old_anm_size = anm.len();
    let pac = if reg::native_stream_mode(&bundle).is_some() {
        Some(disc.read_entry_footprint(map + 4)?)
    } else {
        None
    };
    let full=legaia_engine_scene::model_bank::SceneModelBank::build(&target);
    use legaia_engine_scene::model_bank::ModelSource;
    ensure!(full.sources().iter().all(|source| {
        let entry=match source {ModelSource::BundlePack{entry_idx,..}|ModelSource::BundleSingle{entry_idx,..}|ModelSource::WalkedPack{entry_idx,..}|ModelSource::WalkedSingle{entry_idx,..}|ModelSource::StreamPack{entry_idx,..}|ModelSource::StreamSingle{entry_idx,..}=>*entry_idx as usize};
        entry==map+3||entry==map+4
    }),"Destination has additional model-registration contributors; cumulative actor selector adapter required");
    let source_bundle=index.entry_bytes(map as u32+3)?;
    let source_pac=reg::native_stream_mode(&source_bundle).map(|_|index.entry_bytes(map as u32+4)).transpose()?;
    ensure!(reg::native_model_pool(&source_bundle,source_pac.as_deref().map(|v|v.as_slice())).len()==full.sources().len(),"Destination native actor model pool is not the complete verified carrier walk");
    reject_other_actor_carriers(&target,map)?;
    let active=active_actor_payloads(&bundle,pac.as_deref())?;
    ensure!(active[&3].0&&active[&5].0&&active[&3].1==man&&active[&5].1==anm,"Destination active MAN/ANM is stream-replaced; native streamed dependency writer required");
    let (_, mut vram) = reg::native_texture_sequence(&bundle, pac.as_deref(), limits)?;
    let mut planner = Planner::new(&vram, &field_runtime_reservations())?;
    planner.reserve_written(&assets::static_resources::resolve_actor_context(index,&target)?.vram);
    let mut boot = legaia_tim::Vram::new();
    index
        .system_ui_bundle()?
        .upload_boot_resident_to_vram(&mut boot);
    planner.reserve_written(&boot);
    let party = index.entry_bytes(legaia_actor_formats::field_char_textures::PROT_ENTRY_INDEX)?;
    let party = legaia_actor_formats::field_char_textures::parse(&party)?;
    let mut resident = legaia_tim::Vram::new();
    party.upload_to_vram(&mut resident, false);
    planner.reserve_written(&resident);
    if let Ok(walkers) = legaia_scene_formats::clut_walk::from_scene_bundle(&bundle) {
        ensure!(
            walkers.entries.is_empty(),
            "Active destination palette walker requires dynamic VRAM lifetime adapter"
        );
    }
    let mut closures = BTreeMap::new();
    let mut selections = vec![];
    for r in &requests {
        ensure!(
            r.origin.is_none(),
            "Animated transplant must have a new Reference identity; original actor edits use their existing adapter"
        );
        let adapter = session
            .project
            .base_adapters
            .get(&r.base)
            .context("Animated Base Form native source adapter missing")?;
        ensure!(
            adapter.kind == "npc",
            "Animated Base Form is not a MAN skeletal actor"
        );
        if let Some(frame) = r.properties.get("npc_animation_frame") {
            ensure!(
                matches!(frame, super::form_database::Property::Integer(0)),
                "Nonzero or unknown native initial-frame state"
            );
        }
        if let Some(clip) = r.properties.get("npc_animation_clip") {
            ensure!(
                matches!(clip, super::form_database::Property::Integer(_)),
                "Unknown native animation selection state"
            );
        }
        let state = if let Some(super::form_database::Property::Integer(clip)) =
            r.properties.get("npc_animation_clip")
        {
            Some(State {
                clip: u8::try_from(*clip)?,
                frame: match r.properties.get("npc_animation_frame") {
                    Some(super::form_database::Property::Integer(v)) => usize::try_from(*v)?,
                    _ => 0,
                },
            })
        } else {
            None
        };
        let actor = adapter
            .origin
            .as_ref()
            .and_then(|o| o.man_record)
            .map(|n| n as usize)
            .unwrap_or(adapter.source_ordinal);
        let key = (
            adapter.source_scene.clone(),
            actor,
            state.as_ref().map(|s| (s.clip, s.frame)),
        );
        if !closures.contains_key(&key) {
            closures.insert(
                key.clone(),
                resolve(index, &adapter.source_scene, actor, state.as_ref())?,
            );
        }
        let c = &closures[&key];
        if let Some(hash)=adapter.origin.as_ref().and_then(|o|o.fingerprints.get("MAN")){ensure!(hash==&c.origin.man_sha256,"Animated donor MAN identity changed");}
        if let Some(hash) = adapter
            .origin
            .as_ref()
            .and_then(|o| o.fingerprints.get("TMD"))
        {
            ensure!(
                hash == &c.origin.model_sha256,
                "Animated donor Base Form model fingerprint differs"
            );
        }
        if let Some(super::form_database::Property::Integer(model)) =
            r.properties.get("npc_model_selection")
        {
            ensure!(
                *model == i64::from(c.origin.model_selector),
                "Animation model override requires a separately bound Base Form"
            );
        }
        selections.push(key);
    }
    let mut native_models = BTreeMap::new();
    let mut donors = vec![];
    let mut resource_dependencies=vec![];
    for (key, c) in &closures {
        let mut inventory = NativeAssetPlan::default();
        let id = inventory.register(
            ResourceKind::Tmd,
            &c.model,
            ResourceOrigin {
                scene: c.origin.scene.clone(),
                entry: None,
                offset: None,
            },
            false,
        )?;
        let native = Scene::load(index, &c.origin.scene)?;
        let canonical =
            c.context
                .dependencies(&native, c.origin.model_selector as u16, &c.model)?;
        ensure!(
            canonical.primitives.iter().all(|p| p.unresolved_words == 0),
            "Animated material has unobserved words; dynamic producer required"
        );
        let events = c.context.register_uploads(&mut inventory)?;
        let trace = UploadTrace::verify_native_replay(
            &inventory,
            &events,
            &c.context.vram,
            "B2B canonical B1 native donor upload context",
        )?;
        let relocation = planner.plan_multi(&inventory, &trace, &[id.clone()])?;
        let (registered, registration) =
            super::ps1_export::NativeExportPlan::compile_scene_resources(
                &disc,
                map,
                &inventory,
                &vram,
                &relocation,
                limits,
            )?;
        let selector = registration.selector(&id)?;
        ensure!(
            selector < 0xf0,
            "Animated model selector aliases global/party model class"
        );
        // A separate loader walk checks the actual actor selector's identity.
        let b = registered.read_entry_footprint(map + 3)?;
        let p = reg::native_stream_mode(&b)
            .map(|_| registered.read_entry_footprint(map + 4))
            .transpose()?;
        let (base, _) = reg::native_descriptor_table(&b)?;
        let independent = super::native_loader_trace::trace(&b, base, p.as_deref())?;
        ensure!(
            independent
                .models
                .get(selector as usize)
                .is_some_and(|m| m.objects
                    == legaia_tmd::parse(&c.model)
                        .map(|t| t.objects.len() as u32)
                        .unwrap_or(0)),
            "Registered actor object table mismatch"
        );
        native_models.insert(key.clone(), selector as u8);
        resource_dependencies.push(serde_json::json!({"origin":c.origin,"canonical_static":canonical,"donor_behavior_evidence":c.behavior,"behavior_transplanted":false}));
        donors.push(c.origin.clone());
        disc = registered;
        vram = relocation.apply(&vram)?;
    }
    let clips = selections
        .iter()
        .map(|k| closures[k].clip.clone())
        .collect::<Vec<_>>();
    let (anm, ids) = append_clips(&anm, &clips)?;
    let actors = requests
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let c = &closures[&selections[i]];
            let pivot = super::world_view::editor_rotate_vertex(
                c.pivot,
                [0.; 3],
                [0.; 3],
                r.transform.rotation,
                r.transform.scale,
            );
            let mut native = r.transform;
            native.position = [0, 1, 2].map(|a| r.transform.position[a] - pivot[a]);
            (
                r.key.clone(),
                r.base.clone(),
                native_models[&selections[i]],
                ids[i],
                native,
            )
        })
        .collect::<Vec<_>>();
    let (man, mut seats) = append_actors(&man, &actors)?;
    for (seat,key) in seats.iter_mut().zip(&selections) {seat.origin=Some(closures[key].origin.clone());}
    bundle = disc.read_entry_footprint(map + 3)?;
    let bundle = rewrite_bundle(
        &bundle,
        &BTreeMap::from([(3, man.clone()), (5, anm.clone())]),
        limits.max_encoded,
    )?;
    let min = disc.read_entry_footprint(map + 3)?.len();
    let mut payload = bundle.clone();
    payload.resize(min.max(payload.len()).div_ceil(2048) * 2048, 0);
    ensure!(
        payload.len() <= 404480,
        "Sector-rounded animation resource carrier exceeds fixed arena"
    );
    if disc
        .entry_footprint(map + 3)
        .is_none_or(|n| payload.len() as u64 > n)
    {
        disc.grow_prot_entries(&BTreeMap::from([(map + 3, payload)]))?;
    } else {
        disc.patch_prot_entry(map + 3, 0, &payload)?;
    }
    let check = disc.read_entry_footprint(map + 3)?;
    let (b, h) = reg::native_descriptor_table(&check)?;
    for (kind, expected) in [(3, &man), (5, &anm)] {
        let d = h
            .used()
            .iter()
            .find(|d| d.type_byte == kind)
            .context("Native animation dependency missing from output IMG")?;
        ensure!(
            legaia_lzs::decompress(&check[b + d.data_offset as usize..], d.size as usize)?
                == *expected,
            "Exact animation dependency IMG readback failed"
        );
    }
    let pac = reg::native_stream_mode(&check)
        .map(|_| disc.read_entry_footprint(map + 4))
        .transpose()?;
    let models=reg::native_model_pool(&check,pac.as_deref());
    for seat in &mut seats {seat.registered_model_sha256=Some(digest(&models.get(seat.model as usize).context("Final animated model slot missing")?.bytes));}
    let runtime_profile =
        serde_json::to_value(super::native_load_budget::analyze(&check, pac.as_deref())?)?;
    let reference_transforms = requests
        .iter()
        .map(|r| (r.key.clone(), r.transform))
        .collect();
    Ok((disc,Compilation{resource_dependencies,seats,donors,encoded_bundle_bytes:check.len(),added_animation_bytes:anm.len().saturating_sub(old_anm_size),runtime_profile,reference_transforms,behavior_preserving:false,gameplay_verified:false,limitations:vec!["Donor dialogue, story, motion VM and interactions detached".into(),"Actor heap/render-node capacity and native gameplay remain unverified".into(),"Rebuild from mounted master; inherited exported-IMG animation ownership not inferred".into()]}))
}

#[cfg(test)]
#[path = "native_animation_tests.rs"]
mod tests;

/// Rebind existing collision evidence only after proving that animation changed
/// neither any journalled MAP nor any preexisting model selector/bytes.
pub(super) fn finalize_log(
    log: &str,
    before: &[u8],
    after: &[u8],
    profile: &serde_json::Value,
) -> Result<String> {
    let old = legaia_disc_patch::disc::DiscPatcher::open(before.to_vec())?;
    let new = legaia_disc_patch::disc::DiscPatcher::open(after.to_vec())?;
    let mut result = vec![];
    for line in log.lines() {
        if let Some(text) = line.strip_prefix("WALL_OWNERSHIP_JOURNAL\t") {
            let mut value: serde_json::Value = serde_json::from_str(text)?;
            ensure!(
                value["image_sha256"].as_str() == Some(digest(before).as_str()),
                "Collision metadata is not bound to pre-animation output"
            );
            let cells = value["cells"]
                .as_array()
                .context("Collision journal schema has no verified cell inventory")?;
            for cell in cells {
                let map = usize::try_from(
                    cell["journal"]["map_entry"]
                        .as_u64()
                        .context("Collision journal MAP provenance missing")?,
                )?;
                ensure!(
                    old.read_entry_footprint(map)? == new.read_entry_footprint(map)?,
                    "Animation changed journalled MAP bytes"
                );
                let a = old.read_entry_footprint(map + 3)?;
                let b = new.read_entry_footprint(map + 3)?;
                let ap = reg::native_stream_mode(&a)
                    .map(|_| old.read_entry_footprint(map + 4))
                    .transpose()?;
                let bp = reg::native_stream_mode(&b)
                    .map(|_| new.read_entry_footprint(map + 4))
                    .transpose()?;
                let am = reg::native_model_pool(&a, ap.as_deref());
                let bm = reg::native_model_pool(&b, bp.as_deref());
                ensure!(
                    am.len() <= bm.len() && am.iter().zip(&bm).all(|(a, b)| a.bytes == b.bytes),
                    "Animation changed preexisting collision model witnesses"
                );
            }
            value["image_sha256"] = serde_json::json!(digest(after));
            result.push(format!(
                "WALL_OWNERSHIP_JOURNAL\t{}",
                serde_json::to_string(&value)?
            ));
        } else if line.starts_with("SCENE_RUNTIME_PROFILE\t") {
            result.push(format!(
                "SCENE_RUNTIME_PROFILE\t{}",
                serde_json::to_string(profile)?
            ));
        } else {
            result.push(line.to_string());
        }
    }
    Ok(result.join("\n"))
}
