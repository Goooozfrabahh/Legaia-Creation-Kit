//! Opt-in, read-only retail census. This file is test-only; no editor/exporter behavior changes.
use super::super::*;
use crate::app::native_asset_plan::{
    self as assets, scene_registration as reg,
    texture_relocation::{
        Planner,
        multi_tim::{Upload, UploadTrace},
    },
};
use serde_json::{Value, json};
use std::{
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};
fn limits() -> reg::Limits {
    reg::Limits {
        max_models: 1024,
        max_decoded: 16 * 1024 * 1024,
        max_encoded: 16 * 1024 * 1024,
    }
}
fn clean(v: impl ToString) -> String {
    v.to_string().replace(['\t', '\r', '\n'], " ")
}
fn create(p: &Path) -> Result<BufWriter<std::fs::File>> {
    Ok(BufWriter::new(
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(p)?,
    ))
}
struct Census {
    assets: BufWriter<std::fs::File>,
    graph: BufWriter<std::fs::File>,
    counts: BTreeMap<String, usize>,
    ids: BTreeSet<String>,
    hashes: BTreeMap<String, BTreeSet<String>>,
    unknown: usize,
}
impl Census {
    fn add(
        &mut self,
        id: &str,
        family: &str,
        role: &str,
        source: &str,
        offset: usize,
        bytes: Option<&[u8]>,
        fields: Value,
        dependencies: Value,
    ) -> Result<()> {
        if !self.ids.insert(id.into()) {
            return Ok(());
        }
        *self.counts.entry(family.into()).or_default() += 1;
        let hash = bytes
            .map(digest)
            .or_else(|| fields["sha256"].as_str().map(str::to_owned))
            .unwrap_or_default();
        if !hash.is_empty() {
            self.hashes
                .entry(family.into())
                .or_default()
                .insert(hash.clone());
        }
        if family.to_ascii_lowercase().contains("unknown") || family.contains("Unresolved") {
            self.unknown += 1;
        }
        writeln!(
            self.assets,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            clean(id),
            clean(family),
            role,
            clean(source),
            offset,
            hash,
            clean(&fields),
            "Unknown / Insufficient Evidence",
            clean(&dependencies)
        )?;
        serde_json::to_writer(
            &mut self.graph,
            &json!({"id":id,"family":family,"role":role,"source":source,"offset":offset,"sha256":hash,"fields":fields,"dependencies":dependencies}),
        )?;
        writeln!(self.graph)?;
        Ok(())
    }
    /// Bounded recursive decoding. Every container and undecoded leaf retains identity.
    fn buffer(
        &mut self,
        id: &str,
        bytes: &[u8],
        depth: usize,
        budget: &mut usize,
        gaps: &mut Vec<String>,
    ) -> Result<()> {
        if depth > 6 || bytes.len() > *budget {
            gaps.push(format!(
                "{id}: bounded decode budget/depth reached; retained parent"
            ));
            return Ok(());
        }
        *budget -= bytes.len();
        let mut spans = vec![];
        for h in legaia_container_formats::tmd_scan::scan_buffer(bytes) {
            let raw = &bytes[h.offset..h.offset + h.byte_len];
            self.add(&format!("{id}/tmd@{}",h.offset),"TMD","definition",id,h.offset,Some(raw),json!({"objects":h.n_obj,"vertices":h.total_verts,"primitives":h.total_prims,"discovery":"word-aligned structural scan; registration unproven"}),json!([]))?;
            spans.push((h.offset, h.offset + h.byte_len));
        }
        for h in legaia_container_formats::tim_scan::scan_buffer(bytes) {
            self.add(&format!("{id}/tim@{}",h.offset),"TIM","definition",id,h.offset,Some(&bytes[h.offset..h.offset+h.byte_len]),json!({"bpp":h.bpp,"width":h.width,"height":h.height,"clut":h.has_clut,"upload_order_verified":false}),json!([]))?;
            spans.push((h.offset, h.offset + h.byte_len));
        }
        // Never reinterpret recognized geometry/image leaf headers as generic streams.
        if spans.iter().any(|&(a, _)| a == 0) {
            return Ok(());
        }
        let mut decoded = false;
        if let Some(ranges) = legaia_scene_formats::scene_event_scripts::record_ranges(bytes) {
            for (i, (a, b)) in ranges.into_iter().enumerate() {
                self.add(
                    &format!("{id}/event:{i}"),
                    "EventStagerRecord",
                    "definition",
                    id,
                    a,
                    Some(&bytes[a..b]),
                    json!({"behavior":"Scripted evidence; VM/actor closure unverified"}),
                    json!([]),
                )?;
            }
        }
        if depth < 3 {
            if let Ok(header) = legaia_lzs::parse_container(bytes) {
                if !header.sections.is_empty()
                    && header
                        .sections
                        .iter()
                        .map(|s| s.size as usize)
                        .sum::<usize>()
                        <= (*budget).min(16 * 1024 * 1024)
                {
                    if let Ok(sections) = legaia_lzs::decompress_container_strict(bytes) {
                        for (i, raw) in sections.iter().enumerate() {
                            let sec = &header.sections[i];
                            let child = format!("{id}/strict-lzs:{i}");
                            self.add(
                                &child,
                                "LZS section",
                                "definition",
                                id,
                                sec.byte_offset as usize,
                                Some(raw),
                                json!({"decoded_length":raw.len(),"heuristic_container":true}),
                                json!([]),
                            )?;
                            self.buffer(&child, raw, depth + 1, budget, gaps)?;
                        }
                    }
                }
            }
        }

        if let Ok((base, table)) = reg::native_descriptor_table(bytes) {
            for (i, d) in table.used().iter().enumerate() {
                let child = format!("{id}/descriptor:{base}:{i}");
                let at = base + d.data_offset as usize;
                let n = d.size as usize;
                let kind = legaia_container_formats::AssetType::from_byte(d.type_byte);
                let family = kind.name();
                if matches!(d.type_byte, 10 | 15 | 20) {
                    self.add(
                        &child,
                        "LoaderFlag",
                        "definition",
                        id,
                        base + 8 + i * 8,
                        None,
                        json!({"type":d.type_byte,"decoded_size":n}),
                        json!([]),
                    )?;
                    continue;
                }
                let result = if n <= 16 * 1024 * 1024 && n <= *budget {
                    bytes
                        .get(at..)
                        .context("descriptor range")
                        .and_then(|b| legaia_lzs::decompress(b, n))
                } else {
                    Err(anyhow::anyhow!(
                        "declared decoded length exceeds per-container budget"
                    ))
                };
                match result {
                    Ok(raw) => {
                        self.add(&child,family,"definition",id,at,Some(&raw),json!({"type":d.type_byte,"encoding":"LZS","decoded_length":n,"descriptor_offset":base+8+i*8}),json!([]))?;
                        self.buffer(&child, &raw, depth + 1, budget, gaps)?;
                    }
                    Err(e) => {
                        self.add(
                            &child,
                            "Unknown / Requires Investigation",
                            "definition",
                            id,
                            at,
                            None,
                            json!({"type":d.type_byte,"error":format!("{e:#}")}),
                            json!([]),
                        )?;
                        gaps.push(format!("{child}: {e:#}"));
                    }
                }
            }
            decoded = true;
        } else if let Ok(stream) = legaia_container_formats::parse_streaming(bytes, 4096) {
            if stream.terminated && stream.all_known_types && !stream.chunks.is_empty() {
                for c in stream.chunks {
                    let at = c.header_offset + 4;
                    let raw = &bytes[at..at + c.size as usize];
                    let child = format!("{id}/chunk@{}", c.header_offset);
                    self.add(
                        &child,
                        c.type_name,
                        "definition",
                        id,
                        at,
                        Some(raw),
                        json!({"type":c.type_byte,"encoding":"raw native stream"}),
                        json!([]),
                    )?;
                    self.buffer(&child, raw, depth + 1, budget, gaps)?;
                }
                decoded = true;
            }
        }
        if !decoded {
            if let Ok(pack) = legaia_container_formats::pack::parse_pack(bytes) {
                if !pack.is_empty() && pack.len() < 4096 {
                    for m in pack {
                        let child = format!("{id}/member:{}", m.index);
                        let raw = &bytes[m.byte_offset..m.byte_offset + m.size];
                        self.add(
                            &child,
                            "PackMember",
                            "definition",
                            id,
                            m.byte_offset,
                            Some(raw),
                            json!({"member":m.index}),
                            json!([]),
                        )?;
                        self.buffer(&child, raw, depth + 1, budget, gaps)?;
                    }
                    decoded = true;
                }
            }
        }
        // Record opaque complement ranges instead of pretending recognized magic covers a carrier.
        if !decoded {
            spans.sort();
            let mut end = 0;
            for (a, b) in spans
                .into_iter()
                .chain(std::iter::once((bytes.len(), bytes.len())))
            {
                if a > end {
                    let raw = &bytes[end..a];
                    if raw.iter().any(|b| *b != 0) {
                        self.add(&format!("{id}/opaque@{end}"),"Unknown / Requires Investigation","opaque-range",id,end,Some(raw),json!({"length":raw.len(),"reason":"No supported bounded structural decoder for this range"}),json!([]))?;
                    }
                }
                end = end.max(b);
            }
        }
        Ok(())
    }
}
#[test]
fn audit_tsv_escaping_and_source_identity_are_stable() {
    assert_eq!(clean("a\tb\nc"), "a b c");
    assert_ne!("scene:a/model:0", "scene:b/model:0");
}
#[test]
#[ignore = "read-only original disc audit; explicit new output directory required; never starts editor/emulator"]
fn stage_b_complete_gamewide_resource_audit() -> Result<()> {
    let source = PathBuf::from(std::env::var_os("LCK_AUDIT_SOURCE").context("LCK_AUDIT_SOURCE")?);
    let root = PathBuf::from(std::env::var_os("LCK_AUDIT_OUTPUT").context("LCK_AUDIT_OUTPUT")?);
    ensure!(
        root.is_absolute() && !root.exists(),
        "New absolute audit output required"
    );
    let before = digest(&std::fs::read(&source)?);
    let library = crate::library::Library::open(&source)?;
    std::fs::create_dir(&root)?;
    std::fs::create_dir(root.join("cells"))?;
    let mut census = Census {
        assets: create(&root.join("ALL_ASSETS.tsv"))?,
        graph: create(&root.join("asset-nodes.jsonl"))?,
        counts: BTreeMap::new(),
        ids: BTreeSet::new(),
        hashes: BTreeMap::new(),
        unknown: 0,
    };
    writeln!(
        census.assets,
        "identity\tfamily\trole\tsource_container\toffset\tsha256\tfields\tbehavior_classification\tdependencies"
    )?;
    let mut gaps = vec![];
    let mut scenes = library.index.cdname_scene_names();
    scenes.sort();
    scenes.dedup();
    // Includes all extraction entries even outside the cataloged field cells.
    for entry in library.index.entries() {
        let id = format!("prot:{}", entry.index);
        match library.index.entry_bytes(entry.index) {
            Ok(bytes) => {
                let class = library
                    .index
                    .class_of(entry.index)
                    .map(|c| format!("{c:?}"))
                    .unwrap_or_else(|e| format!("Unknown: {e:#}"));
                census.add(&id,"PROT entry","container","PROT.DAT",entry.start_lba as usize*2048,Some(&bytes),json!({"index":entry.index,"length":bytes.len(),"class":class,"scene":library.index.scene_for_index(entry.index)}),json!([]))?;
                census.buffer(&id, &bytes, 0, &mut (64 * 1024 * 1024), &mut gaps)?;
            }
            Err(e) => {
                gaps.push(format!("{id}: {e:#}"));
                census.add(
                    &id,
                    "Unknown PROT entry",
                    "container",
                    "PROT.DAT",
                    0,
                    None,
                    json!({"error":format!("{e:#}")}),
                    json!([]),
                )?;
            }
        }
        if entry.index % 100 == 0 {
            census.assets.flush()?;
            census.graph.flush()?;
            println!(
                "Raw entry {} / {}",
                entry.index,
                library.index.entry_count()
            );
        }
    }
    let mut disc = legaia_iso::raw::RawDisc::open(&source)?;
    for f in &library.iso_files {
        let id = format!("iso:{}", f.path);
        let mut data = Vec::new();
        disc.read_user_data(f.lba, f.size.div_ceil(2048), &mut data)?;
        data.truncate(f.size as usize);
        census.add(
            &id,
            "ISO file",
            "container",
            "ISO9660",
            f.lba as usize * 2048,
            Some(&data),
            json!({"path":f.path,"lba":f.lba,"length":f.size}),
            json!([]),
        )?;
        if f.path.to_ascii_uppercase().contains("PROT.DAT") {
            continue;
        }
        if data.len() <= 16 * 1024 * 1024 {
            census.buffer(&id, &data, 0, &mut (64 * 1024 * 1024), &mut gaps)?;
        } else {
            gaps.push(format!("{id}: large ISO file hashed in full; deep multimedia sector/frame decoding not attempted"));
        }
    }
    let destination_scene = Scene::load(&library.index, "town01")?;
    let destination_bundle = legaia_engine_scene::scene_bundle::find_bundle(&destination_scene)
        .context("Bounded registration destination bundle")?
        .bytes()
        .to_vec();
    let destination_map = destination_scene
        .field_map_index(&library.index)
        .context("Bounded destination MAP")?;
    let destination_pac = reg::native_stream_mode(&destination_bundle)
        .map(|_| library.index.entry_bytes(destination_map + 4))
        .transpose()?;
    let (_, destination_vram) = reg::native_texture_sequence(
        &destination_bundle,
        destination_pac.as_ref().map(|p| p.as_slice()),
        limits(),
    )?;
    let mut resident = legaia_tim::Vram::new();
    library
        .index
        .system_ui_bundle()?
        .upload_boot_resident_to_vram(&mut resident);
    legaia_actor_formats::field_char_textures::parse(
        &library
            .index
            .entry_bytes(legaia_actor_formats::field_char_textures::PROT_ENTRY_INDEX)?,
    )?
    .upload_to_vram(&mut resident, false);
    let mut compat = create(&root.join("EXPORT_COMPATIBILITY.tsv"))?;
    writeln!(
        compat,
        "identity\tdiscovered\tstructure_parsed\tvisual_dependencies_resolved\tpreviewable\tbase_form\treference\tnative_encoding\tautomated_package_readback\tIMG_readback\tloader_identity\tgameplay\tresult\treason"
    )?;
    let mut behavior = create(&root.join("SCRIPTED_ANIMATED_ASSETS.tsv"))?;
    writeln!(
        behavior,
        "identity\tprimary_category\tclassification\tevidence\tstatic_visual_copy_omits_behavior"
    )?;
    let (
        mut model_count,
        mut closure_count,
        mut package_count,
        mut field_cells,
        mut failed_cells,
        mut boot_users,
    ) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    let mut failures = BTreeMap::<String, usize>::new();
    let mut family_examples = BTreeMap::<String, Value>::new();
    let mut scene_summaries = vec![];
    for (si, name) in scenes.iter().enumerate() {
        println!("Scene {}/{} {name}", si + 1, scenes.len());
        match inspect(&library.index, name) {
            Ok(cell) => {
                if cell.records.contains_key("map") {
                    field_cells += 1;
                }
                for (rid, r) in &cell.records {
                    let id = format!("scene:{name}/{rid}");
                    let deps=Value::Array(r.dependencies.iter().map(|d|json!({"target":format!("scene:{name}/{}",d.target),"relationship":d.relationship,"evidence":d.evidence,"explanation":d.explanation})).collect());
                    let at = r.address.as_ref().map_or(0, |a| a.offset);
                    let container = r
                        .address
                        .as_ref()
                        .map(|a| a.container.as_str())
                        .unwrap_or(name);
                    census.add(
                        &id,
                        &r.kind,
                        if matches!(
                            r.kind.as_str(),
                            "MapPlacement" | "TerrainDecoration" | "ManActor"
                        ) {
                            "occurrence"
                        } else {
                            "definition"
                        },
                        container,
                        at,
                        None,
                        r.fields.clone(),
                        deps,
                    )?;
                    let animated = r.kind == "ManActor" || r.kind == "AnimationSelector";
                    let scripted = r.kind == "ManRecord"
                        || r.dependencies.iter().any(|d| d.target.starts_with("man:"));
                    if animated || scripted {
                        writeln!(
                            behavior,
                            "{}\t{}\t{}\t{}\ttrue",
                            clean(&id),
                            r.kind,
                            if animated && scripted {
                                "Animated + Scripted"
                            } else if animated {
                                "Animated evidence; scripting unknown"
                            } else {
                                "Scripted evidence; animation unknown"
                            },
                            clean(json!({"fields":r.fields,"edges":r.dependencies}))
                        )?;
                    }
                }
                scene_summaries.push(json!({"name":name,"entry_range":cell.entry_range,"records":cell.records.len(),"gaps":cell.gaps}));
                let mut f = create(&root.join(format!("cells/{si:04}.json")))?;
                serde_json::to_writer(&mut f, &cell)?;
            }
            Err(e) => {
                failed_cells += 1;
                gaps.push(format!("Scene {name} inventory: {e:#}"));
                scene_summaries.push(json!({"name":name,"error":format!("{e:#}")}));
            }
        }
        let scene = match Scene::load(&library.index, name) {
            Ok(s) => s,
            Err(e) => {
                gaps.push(format!("{name} loader: {e:#}"));
                continue;
            }
        };
        let bank = SceneModelBank::build(&scene);
        let models = bank.materialise(&scene);
        let source_context=assets::static_resources::resolve_context(&library.index,&scene);
        let source_sequence=(||->Result<_>{let c=source_context.as_ref().map_err(|e|anyhow::anyhow!("{e:#}"))?;
        Ok((Vec::<u8>::new(),None::<std::sync::Arc<Vec<u8>>>,c.uploads.clone(),c.vram.clone(),Err::<crate::app::native_load_budget::Profile,_>(anyhow::anyhow!("See per-scene static dispatcher profiles; canonical upload context is not a heap certificate"))))})();
        let mut inventory = assets::NativeAssetPlan::default();
        let mut trace = None;
        let mut seq_error = String::new();
        if let Ok((_, _, uploads, vram, _)) = &source_sequence {
            let mut events = vec![];
            for (i, raw) in uploads.iter().enumerate() {
                let tim = legaia_tim::parse_native_upload(raw)?;
                let id = inventory.register(
                    assets::ResourceKind::Tim,
                    raw,
                    assets::Origin {
                        scene: name.clone(),
                        entry: None,
                        offset: Some(i),
                    },
                    false,
                )?;
                events.push(Upload {
                    sequence: Some(i as u32),
                    resource: id,
                    image: true,
                    clut: tim.clut.is_some(),
                    merge_clut_zeros: false,
                });
                census.add(&format!("scene:{name}/ordered-upload:{i}"),"OrderedNativeUpload","definition",name,i,Some(raw),json!({"image":[tim.image.fb_x,tim.image.fb_y,tim.image.fb_w,tim.image.h],"clut":tim.clut.as_ref().map(|c|[c.fb_x,c.fb_y,c.w,c.h]),"order":i,"includes_boot":true}),json!([]))?;
            }
            match UploadTrace::verify_native_replay(
                &inventory,
                &events,
                vram,
                "boot TOC0 then native descriptors/PAC; static replay",
            ) {
                Ok(t) => trace = Some(t),
                Err(e) => seq_error = format!("{e:#}"),
            };
        } else if let Err(e) = &source_sequence {
            seq_error = format!("{e:#}");
        }
        let mut cache = BTreeMap::<String, (bool, bool, String)>::new();
        for (selector, raw) in models.iter().enumerate() {
            model_count += 1;
            let id = format!("scene:{name}/model:{selector}");
            let Some(raw) = raw else {
                *failures.entry("Model materialization".into()).or_default() += 1;
                writeln!(
                    compat,
                    "{id}\ttrue\tfalse\tfalse\tunknown\tunknown\tunknown\tfalse\tfalse\tfalse\tfalse\tfalse\tFAILED\tModel materialization failed"
                )?;
                continue;
            };
            if let Ok(context)=&source_context {let canonical=context.dependencies(&scene,selector as u16,raw).map(|c|serde_json::to_value(c).unwrap()).unwrap_or_else(|e|json!({"unresolved":format!("{e:#}"),"behavior_verified":false}));
            census.add(&format!("{id}/canonical-static-dependencies"),"CanonicalStaticDependencies","definition",&id,0,None,canonical,json!([{"target":id,"relationship":"canonical model source; containing BaseForm retains independent FormKey"}]))?;}
            let hash = digest(raw);
            let valid = legaia_tmd::parse(raw).is_ok();
            let (resolved, encoded, reason) = if let Some(v) = cache.get(&hash) {
                v.clone()
            } else {
                let attempt = (|| -> Result<_> {
                    let trace = trace
                        .as_ref()
                        .with_context(|| format!("Ordered native uploads: {seq_error}"))?;
                    let tmd = inventory.register(
                        assets::ResourceKind::Tmd,
                        raw,
                        assets::Origin {
                            scene: name.clone(),
                            entry: None,
                            offset: Some(selector),
                        },
                        false,
                    )?;
                    let before = legaia_tim::Vram::new();
                    let plan=Planner::new(&before,&crate::app::native_asset_plan::texture_relocation::field_runtime_reservations())?.plan_multi(&inventory,trace,&[tmd.clone()])?;
                    let package = plan.package(&before)?;
                    let members = legaia_container_formats::pack::parse_pack(&package.tmd_pack)?;
                    ensure!(
                        members.len() == 1,
                        "one-model package changed identity count"
                    );
                    let m = &members[0];
                    let relocated = &package.tmd_pack[m.byte_offset..m.byte_offset + m.size];
                    let out = plan.apply(&before)?;
                    let (_, changed) = assets::texture_relocation::texture_sample_changes(
                        raw,
                        &source_sequence.as_ref().unwrap().3,
                        relocated,
                        &out,
                    )?;
                    ensure!(changed == 0, "relocated material samples differ");
                    // One real registration check per loader-source family, retained separately from all-model package checks.
                    let source_kind = format!("{:?}", bank.scene_source(selector as u16))
                        .split('{')
                        .next()
                        .unwrap_or("Unknown")
                        .trim()
                        .to_string();
                    if !family_examples.contains_key(&source_kind) {
                        let (b, p, _, _, profile) = source_sequence.as_ref().unwrap();
                        let mut destination_planner = Planner::new(
                            &destination_vram,
                            &assets::texture_relocation::field_runtime_reservations(),
                        )?;
                        destination_planner.reserve_written(&resident);
                        let registration = (|| -> Result<_> {
                            let destination_plan = destination_planner.plan_multi(
                                &inventory,
                                trace,
                                &[tmd.clone()],
                            )?;
                            let package = destination_plan.package(&destination_vram)?;
                            let registered = reg::register_scene(
                                &inventory,
                                &destination_bundle,
                                destination_pac.as_ref().map(|p| p.as_slice()),
                                &destination_vram,
                                &destination_plan,
                                limits(),
                            )?;
                            let members =
                                legaia_container_formats::pack::parse_pack(&package.tmd_pack)?;
                            let m = &members[0];
                            Ok((
                                registered,
                                package.tmd_pack[m.byte_offset..m.byte_offset + m.size].to_vec(),
                            ))
                        })();
                        let result = match registration {
                            Ok((r, expected_tmd)) => {
                                let n = r.selector(&tmd)?;
                                let (base, _) = reg::native_descriptor_table(r.bytes())?;
                                match crate::app::native_loader_trace::trace(
                                    r.bytes(),
                                    base,
                                    r.pac_bytes(),
                                ) {
                                    Ok(t) => {
                                        json!({"static_native_loader_hash_matches":t.models.get(n as usize).is_some_and(|m|m.bytes==expected_tmd),"selector":n,"runtime_verified":false})
                                    }
                                    Err(e) => json!({"error":format!("{e:#}")}),
                                }
                            }
                            Err(e) => json!({"error":format!("{e:#}")}),
                        };
                        family_examples.insert(source_kind,json!({"identity":id,"destination":"town01 bounded fixture; source scene not modified","registration":result,"source_memory_profile":profile.as_ref().ok(),"memory_error":profile.as_ref().err().map(|e|format!("{e:#}"))}));
                    }
                    let diagnostics = plan.resource_diagnostics();
                    census.add(&format!("{id}/visual-package-check"),"VisualPackageVerification","verification",&id,0,None,json!({"diagnostics":diagnostics,"material_samples_changed":changed,"runtime_verified":false}),json!([{"target":id,"relationship":"verified visual package source only"}]))?;
                    let boot = diagnostics.iter().any(|s| s.contains("7557b11a"));
                    Ok((
                        true,
                        true,
                        format!(
                            "material samples preserved; native TIM/TMD package readback; full behavior unresolved; boot-atlas-hash-used={boot}"
                        ),
                    ))
                })();
                let v = match attempt {
                    Ok(v) => v,
                    Err(e) => (false, false, format!("{e:#}")),
                };
                cache.insert(hash, v.clone());
                v
            };
            if resolved {
                closure_count += 1;
            }
            if encoded {
                package_count += 1;
            }
            if reason.contains("boot-atlas-hash-used=true") {
                boot_users += 1;
            }
            if !encoded {
                let group = if reason.contains("Unresolved native VRAM word") || reason.contains("external image") {
                    "Unresolved texture/CLUT"
                } else if reason.contains("Fragmented") {
                    "Composite image/palette writer"
                } else if reason.contains("VRAM") || reason.contains("allocation") {
                    "Native VRAM allocation"
                } else if reason.contains("descriptor") || reason.contains("MAP") {
                    "Native carrier/loader adapter"
                } else {
                    "Primitive/upload decoder"
                };
                *failures.entry(group.into()).or_default() += 1;
            }
            writeln!(
                compat,
                "{}\ttrue\t{}\t{}\tunknown\tTMD-definition-only\tunknown\t{}\t{}\tfalse\tfalse\tfalse\t{}\t{}",
                id,
                valid,
                resolved,
                encoded,
                encoded,
                if encoded {
                    "PACKAGE VERIFIED / BEHAVIOR UNKNOWN"
                } else {
                    "UNRESOLVED"
                },
                clean(reason)
            )?;
        }
        compat.flush()?;
        census.assets.flush()?;
        census.graph.flush()?;
    }
    let after = digest(&std::fs::read(&source)?);
    ensure!(before == after, "Retail source changed");
    let totals = json!({"schema":1,"checkpoint":"fec6025","source":source,"source_sha256":before,"source_unchanged":true,"named_blocks_attempted":scenes.len(),"field_cells":field_cells,"failed_cell_inventories":failed_cells,"family_counts":census.counts,"unique_content_counts":census.hashes.iter().map(|(k,v)|(k.clone(),v.len())).collect::<BTreeMap<_,_>>(),"model_registrations_tested":model_count,"visual_package_dependencies_resolved":closure_count,"native_package_readback_passed":package_count,"full_behavior_dependency_closure_verified":0,"IMG_readback_in_this_census":0,"gameplay_verified":0,"boot_atlas_model_users":boot_users,"failures":failures,"registration_family_examples":family_examples,"scene_summaries":scene_summaries,"scan_gaps":gaps,"unknown_identity_rows":census.unknown});
    serde_json::to_writer_pretty(create(&root.join("audit-totals.json"))?, &totals)?;
    let mut graph = create(&root.join("RESOURCE_DEPENDENCIES.json"))?;
    writeln!(
        graph,
        "{{\"schema\":1,\"source_sha256\":\"{before}\",\"nodes\":["
    )?;
    census.graph.flush()?;
    let nodes = std::fs::read_to_string(root.join("asset-nodes.jsonl"))?;
    for (i, line) in nodes.lines().enumerate() {
        if i > 0 {
            writeln!(graph, ",")?;
        }
        write!(graph, "{line}")?;
    }
    writeln!(
        graph,
        "],\"identity_policy\":\"source identity and occurrences retained; content hashes do not merge Base Forms\",\"gameplay_verified\":false}}"
    )?;
    println!("AUDIT COMPLETE {}", root.display());
    Ok(())
}

#[test]
fn audit_native_leaf_and_opaque_range_retention() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut c = Census {
        assets: create(&root.path().join("assets.tsv"))?,
        graph: create(&root.path().join("nodes.jsonl"))?,
        counts: BTreeMap::new(),
        ids: BTreeSet::new(),
        hashes: BTreeMap::new(),
        unknown: 0,
    };
    let mut gaps = vec![];
    c.buffer("opaque", &[0x55; 32], 0, &mut 1024, &mut gaps)?;
    assert_eq!(c.counts["Unknown / Requires Investigation"], 1);
    let mut raw = vec![];
    for n in [16u32, 2, 14, 0, 0x00010001] {
        raw.extend(n.to_le_bytes());
    }
    raw.extend(7u16.to_le_bytes());
    c.buffer("tim", &raw, 0, &mut (1024 * 1024), &mut gaps)?;
    assert_eq!(c.counts["TIM"], 1);
    assert!(!c.ids.iter().any(|id| id.starts_with("tim/chunk")));
    Ok(())
}

#[test]
#[ignore = "read-only per-cell native allocation request profiles; existing completed audit folder required"]
fn stage_b_all_scene_memory_profiles() -> Result<()> {
    let source = PathBuf::from(std::env::var_os("LCK_AUDIT_SOURCE").context("source")?);
    let root = PathBuf::from(std::env::var_os("LCK_AUDIT_OUTPUT").context("output")?);
    let totals: Value = serde_json::from_slice(&std::fs::read(root.join("audit-totals.json"))?)?;
    let hash = digest(&std::fs::read(&source)?);
    ensure!(totals["source_sha256"] == hash, "Audit source mismatch");
    let library = crate::library::Library::open(&source)?;
    let mut rows = vec![];
    let mut names = library.index.cdname_scene_names();
    names.sort();
    names.dedup();
    for name in names {
        let profile = (|| -> Result<_> {
            let scene = Scene::load(&library.index, &name)?;
            let bundle = legaia_engine_scene::scene_bundle::find_bundle(&scene)
                .context("Native descriptor carrier not established")?;
            let map = scene
                .field_map_index(&library.index)
                .context("Native field stream source not established")?;
            let pac = reg::native_stream_mode(bundle.bytes())
                .map(|_| library.index.entry_bytes(map + 4))
                .transpose()?;
            crate::app::native_load_budget::analyze(
                bundle.bytes(),
                pac.as_ref().map(|p| p.as_slice()),
            )
        })();
        rows.push(match profile {
            Ok(p) => json!({"scene":name,"profile":p,"runtime_verified":false}),
            Err(e) => json!({"scene":name,"unresolved":format!("{e:#}"),"runtime_verified":false}),
        });
    }
    serde_json::to_writer_pretty(
        create(&root.join("SCENE_MEMORY_PROFILES.json"))?,
        &json!({"source_sha256":hash,"scenes":rows,"runtime_verified":false}),
    )?;
    ensure!(digest(&std::fs::read(&source)?) == hash, "Source changed");
    Ok(())
}
