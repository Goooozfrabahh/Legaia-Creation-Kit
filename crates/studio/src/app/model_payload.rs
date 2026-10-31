//! A native type tag is a dispatch instruction, not proof of a bare model.
//! Preserve containers and undecoded candidates without inventing placeability.
use crate::app::collision_recipes::digest;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::app) struct ModelMember {
    pub member: usize,
    pub offset: usize,
    pub stored_size: usize,
    pub geometry_size: usize,
    pub geometry_sha256: String,
    pub objects: usize,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub(in crate::app) enum ModelPayload {
    BareModel {
        geometry_size: usize,
        geometry_sha256: String,
        objects: usize,
    },
    ModelPack {
        members: Vec<ModelMember>,
    },
    Unresolved {
        reason: String,
    },
}
/// Structural evidence only. Native registration, actor class, animation,
/// textures and object visibility require separate native consumer evidence.
pub(in crate::app) fn inspect(raw: &[u8]) -> ModelPayload {
    let bare = |data: &[u8]| -> Option<(usize, String, usize)> {
        let p = legaia_tmd::parse(data).ok()?;
        if p.objects.is_empty() {
            return None;
        }
        let mut size = legaia_tmd::HEADER_SIZE + p.objects.len() * legaia_tmd::OBJECT_SIZE;
        for o in &p.objects {
            size = size
                .max(
                    legaia_tmd::HEADER_SIZE
                        + o.header.vert_top as usize
                        + o.header.n_vert as usize * legaia_tmd::VECTOR_SIZE,
                )
                .max(
                    legaia_tmd::HEADER_SIZE
                        + o.header.normal_top as usize
                        + o.header.n_normal as usize * legaia_tmd::VECTOR_SIZE,
                )
                .max(o.primitives_byte_offset + o.primitives_byte_size);
        }
        Some((size, digest(data.get(..size)?), p.objects.len()))
    };
    if let Some((geometry_size, geometry_sha256, objects)) = bare(raw) {
        return ModelPayload::BareModel {
            geometry_size,
            geometry_sha256,
            objects,
        };
    }
    if let Ok(pack) = legaia_container_formats::pack::parse_pack(raw) {
        if !pack.is_empty() && pack.len() <= 4096 {
            let mut members = Vec::new();
            for m in pack {
                let Some(data) = raw.get(m.byte_offset..m.byte_offset + m.size) else {
                    return ModelPayload::Unresolved {
                        reason: "Model pack member outside carrier".into(),
                    };
                };
                let Some((geometry_size, geometry_sha256, objects)) = bare(data) else {
                    return ModelPayload::Unresolved{reason:"Pack-shaped source contains non-TMD members; native consumer/type remains unresolved".into()};
                };
                members.push(ModelMember {
                    member: m.index,
                    offset: m.byte_offset,
                    stored_size: m.size,
                    geometry_size,
                    geometry_sha256,
                    objects,
                });
            }
            if !members.is_empty() {
                return ModelPayload::ModelPack { members };
            }
        }
    }
    ModelPayload::Unresolved{reason:"Dispatch tag/structural scan context is not a valid bare TMD or complete native TMD pack".into()}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dispatch_tags_never_prove_model_geometry() {
        assert!(matches!(inspect(&[0; 32]), ModelPayload::Unresolved { .. }));
        let raw = [1u32.to_le_bytes(), 2u32.to_le_bytes(), 0u32.to_le_bytes()].concat();
        assert!(matches!(inspect(&raw), ModelPayload::Unresolved { .. }));
    }
    #[test]
    fn geometry_extent_and_pack_container_identities_remain_distinct() {
        let mut model = Vec::new();
        for word in [0x80000002u32, 0, 1, 28, 4, 60, 0, 60, 0, 0x808080] {
            model.extend(word.to_le_bytes());
        }
        model.resize(72, 0);
        let mut padded = model.clone();
        padded.extend([0; 4]);
        let ModelPayload::BareModel {
            geometry_sha256,
            geometry_size,
            ..
        } = inspect(&padded)
        else {
            panic!("valid model")
        };
        assert_eq!(geometry_size, 72);
        assert_eq!(geometry_sha256, digest(&model));
        assert_ne!(digest(&padded), digest(&model));
        let mut pack = Vec::new();
        for word in [2u32, 3, 22] {
            pack.extend(word.to_le_bytes());
        }
        pack.extend(&padded);
        pack.extend(&model);
        let ModelPayload::ModelPack { members } = inspect(&pack) else {
            panic!("native model pack")
        };
        assert_eq!(members.len(), 2);
        assert_eq!(members[0].geometry_sha256, members[1].geometry_sha256);
        assert_ne!(members[0].offset, members[1].offset);
        assert_ne!(members[0].stored_size, members[1].stored_size);
    }
}
