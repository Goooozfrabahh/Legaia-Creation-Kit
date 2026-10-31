//! Canonical static dependencies. The containing BaseForm supplies FormKey identity;
//! payload equality never merges forms, references, provenance or behavior.
use super::{NativeAssetPlan, Origin, ResourceKind, scene_registration as reg};
use crate::app::collision_recipes::digest;
use anyhow::{Context, Result, ensure};
use legaia_engine_scene::{
    model_bank::{ModelSource, SceneModelBank},
    scene::{ProtIndex, Scene},
};
use legaia_tim::Vram;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::app) struct Address {
    pub namespace: String,
    pub entry: Option<u32>,
    pub stored_offset: usize,
    pub decoded_offset: usize,
    pub original_sha256: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::app) struct UploadRecord {
    pub sequence: usize,
    pub source: Address,
    pub upload_sha256: String,
    pub image: [u16; 4],
    pub clut: Option<[u16; 4]>,
    pub upload_depth: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::app) struct Primitive {
    pub offset: usize,
    pub textured: bool,
    pub tsb: Option<u16>,
    pub cba: Option<u16>,
    pub sampled_depth: Option<u8>,
    pub uvs: Vec<(u8, u8)>,
    pub image_words: Option<[u16; 4]>,
    pub palette_words: Option<[u16; 4]>,
    #[serde(default)]
    pub word_reads: Vec<serde_json::Value>,
    #[serde(default)]
    pub unresolved_words: usize,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::app) struct Dependencies {
    pub schema: u32,
    pub scene: String,
    pub selector: u16,
    pub registration: serde_json::Value,
    pub tmd_sha256: String,
    pub primitives: Vec<Primitive>,
    pub ordered_uploads: Vec<UploadRecord>,
    pub carriers: Vec<serde_json::Value>,
    pub unresolved: Vec<String>,
    pub full_behavior_verified: bool,
}
pub(in crate::app) struct ContextData {
    pub scene: String,
    pub uploads: Vec<Vec<u8>>,
    pub records: Vec<UploadRecord>,
    pub vram: Vram,
    pub registrations: Vec<ModelSource>,
    pub models: Vec<Option<Vec<u8>>>,
    pub carriers: Vec<serde_json::Value>,
    pub word_sources: Vec<u32>,
}
fn entry_of(s: ModelSource) -> u32 {
    match s {
        ModelSource::BundlePack { entry_idx, .. }
        | ModelSource::BundleSingle { entry_idx, .. }
        | ModelSource::WalkedPack { entry_idx, .. }
        | ModelSource::WalkedSingle { entry_idx, .. }
        | ModelSource::StreamPack { entry_idx, .. }
        | ModelSource::StreamSingle { entry_idx, .. } => entry_idx,
    }
}
fn registration_record(source: ModelSource) -> serde_json::Value {
    match source {
        ModelSource::BundlePack {
            entry_idx,
            descriptor,
            member,
        } => {
            serde_json::json!({"kind":"BundlePack","entry":entry_idx,"descriptor":descriptor,"member":member})
        }
        ModelSource::WalkedPack {
            entry_idx,
            descriptor,
            member,
        } => {
            serde_json::json!({"kind":"WalkedPack","entry":entry_idx,"descriptor":descriptor,"member":member})
        }
        ModelSource::BundleSingle {
            entry_idx,
            descriptor,
        } => serde_json::json!({"kind":"BundleSingle","entry":entry_idx,"descriptor":descriptor}),
        ModelSource::WalkedSingle {
            entry_idx,
            descriptor,
        } => serde_json::json!({"kind":"WalkedSingle","entry":entry_idx,"descriptor":descriptor}),
        ModelSource::StreamPack {
            entry_idx,
            chunk_header,
            member,
        } => {
            serde_json::json!({"kind":"StreamPack","entry":entry_idx,"chunk_header":chunk_header,"member":member})
        }
        ModelSource::StreamSingle {
            entry_idx,
            chunk_header,
        } => {
            serde_json::json!({"kind":"StreamSingle","entry":entry_idx,"chunk_header":chunk_header})
        }
    }
}
fn push(out: &mut ContextData, raw: Vec<u8>, source: Address) -> Result<()> {
    let tim = legaia_tim::parse_native_upload(&raw)?;
    out.records.push(UploadRecord {
        sequence: out.uploads.len(),
        source,
        upload_sha256: digest(&raw),
        image: [tim.image.fb_x, tim.image.fb_y, tim.image.fb_w, tim.image.h],
        clut: tim.clut.as_ref().map(|c| [c.fb_x, c.fb_y, c.w, c.h]),
        upload_depth: tim.flags & 3,
    });
    let seq = (out.uploads.len() as u32) * 2;
    if let Some(c) = &tim.clut {
        for i in 0..c.entries.len() {
            let x = c.fb_x as usize + i;
            let y = c.fb_y as usize;
            if x < 1024 && y < 512 {
                out.word_sources[y * 1024 + x] = seq + 1;
            }
        }
    }
    for y in 0..tim.image.h as usize {
        for x in 0..tim.image.fb_w as usize {
            let xx = tim.image.fb_x as usize + x;
            let yy = tim.image.fb_y as usize + y;
            if xx < 1024 && yy < 512 {
                out.word_sources[yy * 1024 + xx] = seq;
            }
        }
    }
    out.vram.upload_tim(&tim);
    out.uploads.push(raw);
    Ok(())
}
fn payload(out: &mut ContextData, kind: u8, bytes: &[u8], source: Address) -> Result<()> {
    match kind {
        0 => {
            let t = legaia_tim::parse_native_upload(bytes)?;
            push(out, bytes[..t.byte_extent()].to_vec(), source)?;
        }
        1 => {
            let members = legaia_container_formats::pack::parse_pack(bytes)?;
            let normalized = reg::native_tim_members(bytes)?;
            ensure!(
                members.len() == normalized.len(),
                "Upload-member identity changed"
            );
            for (m, raw) in members.iter().zip(normalized) {
                let mut origin = source.clone();
                origin.decoded_offset += m.byte_offset;
                origin.original_sha256 = digest(&bytes[m.byte_offset..m.byte_offset + m.size]);
                push(out, raw, origin)?;
            }
        }
        _ => {}
    }
    Ok(())
}
/// The same count-driven descriptor and typed-stream families as the native
/// model registrar, never a scene-name whitelist or a magic-hit upload order.
pub(in crate::app) fn resolve_context(index: &ProtIndex, scene: &Scene) -> Result<ContextData> {
    let mut out = ContextData {
        scene: scene.name.clone(),
        uploads: vec![],
        records: vec![],
        vram: Vram::new(),
        registrations: vec![],
        models: vec![],
        carriers: vec![],
        word_sources: vec![u32::MAX; 1024 * 512],
    };
    let boot = index.system_ui_bundle()?;
    let toc = index.toc();
    let a = *toc.first().context("Raw boot TOC missing")? as u64 * 2048;
    let b = *toc.get(1).context("Raw boot end missing")? as u64 * 2048;
    let boot_bytes = index.prot_dat_raw_bytes(a, usize::try_from(b - a)?)?;
    out.carriers.push(serde_json::json!({"namespace":"raw-toc:0","sha256":digest(&boot_bytes),"length":boot_bytes.len()}));
    let (uploads, boot_vram) = reg::native_boot_texture_sequence(index)?;
    for (i, raw) in uploads.into_iter().enumerate() {
        let offset = boot
            .tims
            .iter()
            .find(|m| m.raw_entry == 0 && m.member == i)
            .map(|m| m.entry_offset)
            .or_else(|| {
                boot.row_patches
                    .iter()
                    .find(|p| p.raw_entry == 0 && p.member == i)
                    .map(|p| p.entry_offset)
            })
            .context("Boot member provenance missing")?;
        let size = boot
            .tims
            .iter()
            .find(|m| m.raw_entry == 0 && m.member == i)
            .map(|m| m.tim.byte_extent())
            .or_else(|| {
                boot.row_patches
                    .iter()
                    .find(|p| p.raw_entry == 0 && p.member == i)
                    .map(|p| 20 + p.data.len())
            })
            .context("Boot source size missing")?;
        let hash = digest(
            boot_bytes
                .get(offset..offset + size)
                .context("Boot member leaves raw entry")?,
        );
        push(
            &mut out,
            raw,
            Address {
                namespace: "raw-toc:0".into(),
                entry: None,
                stored_offset: offset,
                decoded_offset: 0,
                original_sha256: hash,
            },
        )?;
    }
    ensure!(
        crate::app::naming::material_key(&out.vram) == crate::app::naming::material_key(&boot_vram),
        "Boot context replay changed"
    );
    let bank = SceneModelBank::build(scene);
    let descriptor = bank
        .sources()
        .iter()
        .copied()
        .find(|s| {
            matches!(
                s,
                ModelSource::BundlePack { .. }
                    | ModelSource::BundleSingle { .. }
                    | ModelSource::WalkedPack { .. }
                    | ModelSource::WalkedSingle { .. }
            )
        })
        .map(entry_of);
    // A descriptor bank with no models can still supply the stream's textures.
    let descriptor = descriptor
        .or_else(|| legaia_engine_scene::scene_bundle::find_bundle(scene).map(|b| b.entry_idx()));
    if let Some(entry) = descriptor {
        let bytes = index.entry_bytes(entry)?;
        out.carriers.push(serde_json::json!({"namespace":"PROT entry","entry":entry,"sha256":digest(&bytes),"length":bytes.len()}));
        let (base, table) = reg::native_descriptor_table(&bytes)?;
        for d in table.used() {
            if !matches!(d.type_byte, 0 | 1) {
                continue;
            }
            ensure!(
                d.size <= 16 * 1024 * 1024,
                "Declared upload exceeds bounded decoder"
            );
            let start = base + d.data_offset as usize;
            let raw = legaia_lzs::decompress(
                bytes
                    .get(start..)
                    .context("Descriptor outside native carrier")?,
                d.size as usize,
            )?;
            payload(
                &mut out,
                d.type_byte,
                &raw,
                Address {
                    namespace: "decoded descriptor".into(),
                    entry: Some(entry),
                    stored_offset: start,
                    decoded_offset: 0,
                    original_sha256: digest(&raw),
                },
            )?;
        }
    }
    // Flag 0x0A selects the native untyped tim.dat word-offset list in
    // the established MAP+4 field streaming slot, rather than typed PAC chunks.
    if let Some(entry) = descriptor {
        let bytes = index.entry_bytes(entry)?;
        if reg::native_stream_mode(&bytes) == Some(0x0a) {
            let map = scene
                .field_map_index(index)
                .context("Flag0A native MAP load anchor unresolved")?;
            let stream_entry = map.checked_add(4).context("Flag0A source overflow")?;
            ensure!(
                scene.entries.iter().any(|e| e.idx == stream_entry),
                "Flag0A tim.dat lies outside native block"
            );
            let data = index.entry_bytes(stream_entry)?;
            out.carriers.push(serde_json::json!({"namespace":"flag0A tim.dat","entry":stream_entry,"sha256":digest(&data),"length":data.len()}));
            payload(
                &mut out,
                1,
                &data,
                Address {
                    namespace: "flag0A native TIM list".into(),
                    entry: Some(stream_entry),
                    stored_offset: 0,
                    decoded_offset: 0,
                    original_sha256: digest(&data),
                },
            )?;
        }
    }
    for entry in &scene.entries {
        if Some(entry.idx) == descriptor
            || entry.class != legaia_disc_audit::categorize::Class::DataFieldStreaming
        {
            continue;
        }
        out.carriers.push(serde_json::json!({"namespace":"PROT entry","entry":entry.idx,"sha256":digest(&entry.bytes),"length":entry.bytes.len()}));
        let stream = legaia_container_formats::parse_streaming(&entry.bytes, 4096)?;
        ensure!(stream.terminated, "Unterminated native upload stream");
        for c in stream.chunks {
            let at = c.header_offset + 4;
            let bytes = entry
                .bytes
                .get(at..at + c.size as usize)
                .context("Stream payload outside entry")?;
            payload(
                &mut out,
                c.type_byte,
                bytes,
                Address {
                    namespace: "native typed stream".into(),
                    entry: Some(entry.idx),
                    stored_offset: at,
                    decoded_offset: 0,
                    original_sha256: digest(bytes),
                },
            )?;
        }
    }
    ensure!(
        !bank.is_empty(),
        "No established model registration; raw candidates remain unresolved"
    );
    out.models = bank.materialise(scene);
    out.registrations = bank.sources().to_vec();
    Ok(out)
}
impl ContextData {
    pub fn register_uploads(
        &self,
        inventory: &mut NativeAssetPlan,
    ) -> Result<Vec<super::texture_relocation::multi_tim::Upload>> {
        let mut events = vec![];
        for (raw, record) in self.uploads.iter().zip(&self.records) {
            let tim = legaia_tim::parse_native_upload(raw)?;
            let resource = inventory.register(
                ResourceKind::Tim,
                raw,
                Origin {
                    scene: self.scene.clone(),
                    entry: record.source.entry,
                    offset: Some(record.source.stored_offset),
                },
                false,
            )?;
            events.push(super::texture_relocation::multi_tim::Upload {
                sequence: Some(record.sequence as u32),
                resource,
                image: true,
                clut: tim.clut.is_some(),
                merge_clut_zeros: false,
            });
        }
        Ok(events)
    }
    pub fn dependencies(&self, scene: &Scene, selector: u16, raw: &[u8]) -> Result<Dependencies> {
        let source = *self
            .registrations
            .get(selector as usize)
            .context("No native registration identity")?;
        let actual = self
            .models
            .get(selector as usize)
            .and_then(|m| m.as_ref())
            .context("Missing scene model")?;
        ensure!(
            actual.as_slice() == raw,
            "Canonical selector does not match donor geometry"
        );
        let parsed = legaia_tmd::parse(raw)?;
        let mut primitives = vec![];
        let mut unresolved=vec!["Animation/script/collision and runtime lifetime closure not certified by static material dependencies".to_string()];
        for object in parsed.objects {
            for group in legaia_tmd::legaia_prims::iter_groups(
                raw,
                object.primitives_byte_offset,
                object.primitives_byte_size,
            )? {
                let desc = legaia_tmd::descriptor::Descriptor::for_flags(group.header.flags)
                    .context("Unknown native primitive descriptor")?;
                for p in group.prims {
                    let textured = desc.packet_shape.is_textured();
                    let depth = ((p.tsb >> 7) & 3) as u8;
                    let (image_words, palette_words) = if textured {
                        ensure!(depth < 3, "Reserved texture depth");
                        let div = match depth {
                            0 => 4,
                            1 => 2,
                            _ => 1,
                        };
                        let minx = p
                            .uvs
                            .iter()
                            .map(|p| p.0 as u16 / div)
                            .min()
                            .context("Missing UV")?;
                        let maxx = p.uvs.iter().map(|p| p.0 as u16 / div).max().unwrap();
                        let y = p.uvs.iter().map(|p| p.1 as u16).min().unwrap();
                        let bottom = p.uvs.iter().map(|p| p.1 as u16).max().unwrap();
                        (
                            Some([
                                (p.tsb & 15) * 64 + minx,
                                ((p.tsb >> 4) & 1) * 256 + y,
                                maxx - minx + 1,
                                bottom - y + 1,
                            ]),
                            if depth < 2 {
                                Some([
                                    (p.cba & 63) * 16,
                                    (p.cba >> 6) & 511,
                                    if depth == 0 { 16 } else { 256 },
                                    1,
                                ])
                            } else {
                                None
                            },
                        )
                    } else {
                        (None, None)
                    };
                    let mut reads = vec![];
                    let mut missing = 0;
                    for (role, rect) in [("image", image_words), ("CLUT", palette_words)] {
                        if let Some([x, y, w, h]) = rect {
                            let mut sources = std::collections::BTreeMap::<u32, usize>::new();
                            let mut missing_here = 0;
                            let mut first_missing = None;
                            for yy in y..y + h {
                                for xx in x..x + w {
                                    let code = if xx < 1024 && yy < 512 {
                                        self.word_sources[yy as usize * 1024 + xx as usize]
                                    } else {
                                        u32::MAX
                                    };
                                    if code == u32::MAX {
                                        missing_here += 1;
                                        first_missing.get_or_insert([xx, yy]);
                                    } else {
                                        *sources.entry(code).or_default() += 1;
                                    }
                                }
                            }
                            for (code, count) in sources {
                                reads.push(serde_json::json!({"request":role,"sequence":code/2,"upload_block":if code%2==0{"image"}else{"CLUT"},"words":count}));
                            }
                            if missing_here > 0 {
                                unresolved.push(format!("Primitive {:#x} {role} rect ({x},{y},{w},{h}): {missing_here} unobserved words; first {first_missing:?}; native runtime/shared source requires investigation",p.bytes_offset));
                            }
                            missing += missing_here;
                        }
                    }
                    primitives.push(Primitive {
                        word_reads: reads,
                        unresolved_words: missing,
                        offset: p.bytes_offset,
                        textured,
                        tsb: textured.then_some(p.tsb),
                        cba: textured.then_some(p.cba),
                        sampled_depth: textured.then_some(depth),
                        uvs: if textured { p.uvs } else { vec![] },
                        image_words,
                        palette_words,
                    });
                }
            }
        }
        Ok(Dependencies {
            schema: 1,
            scene: self.scene.clone(),
            selector,
            registration: registration_record(source),
            tmd_sha256: digest(raw),
            primitives,
            ordered_uploads: self.records.clone(),
            carriers: self.carriers.clone(),
            unresolved,
            full_behavior_verified: false,
        })
    }
}

impl crate::app::form_database::BaseForm {
    /// Explicit per-form binding; FormKey ownership remains with this BaseForm.
    /// No behavior-complete bit, native reference or existing identity is changed.
    pub fn resolve_static_resources(
        &mut self,
        context: &ContextData,
        scene: &Scene,
        selector: u16,
    ) -> Result<()> {
        let raw = context
            .models
            .get(selector as usize)
            .and_then(|m| m.as_ref())
            .context("Unresolved model bytes")?;
        if let Some(origin) = &self.origin {
            if let Some(name) = &origin.scene {
                ensure!(
                    name == &scene.name,
                    "Base Form source scene differs from registration evidence"
                );
            }
        }
        self.static_resources = Some(context.dependencies(scene, selector, raw)?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::form_database::*;
    #[test]
    fn canonical_dependencies_preserve_form_identity_and_old_projects() {
        let key = FormKey::new(FileId::new("fixture").unwrap(), 1).unwrap();
        let mut base = BaseForm::empty(
            key.clone(),
            "Architecture".into(),
            FormKind::StaticEnvironment,
        );
        let old = serde_json::to_value(&base).unwrap();
        assert!(old.get("static_resources").is_none());
        let loaded: BaseForm = serde_json::from_value(old).unwrap();
        assert!(loaded.static_resources.is_none());
        base.static_resources = Some(Dependencies {
            schema: 1,
            scene: "native-fixture".into(),
            selector: 3,
            registration: serde_json::json!({"entry":7,"member":3}),
            tmd_sha256: "same-payload".into(),
            primitives: vec![],
            ordered_uploads: vec![],
            carriers: vec![],
            unresolved: vec!["behavior".into()],
            full_behavior_verified: false,
        });
        let mut other = base.clone();
        other.key = FormKey::new(key.file.clone(), 2).unwrap();
        assert_ne!(base.key, other.key);
        assert_eq!(base.static_resources, other.static_resources);
        assert!(!base.dependency_coverage_complete);
        let readback: BaseForm =
            serde_json::from_slice(&serde_json::to_vec(&base).unwrap()).unwrap();
        assert_eq!(base, readback);
    }
}
