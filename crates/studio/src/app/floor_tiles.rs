//! Original PS1 square-floor candidates for the LCK World Repository.
//! Read-only resource indexing. No synthetic textures or inferred walkability.

use std::collections::{BTreeMap, BTreeSet};
use eframe::egui;
use crate::raster::Model;
use super::global_repository::{Asset, Kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Face {
    pub object: usize,
    pub primitive: usize,
    pub y: i16,
    pub cba: u16,
    pub tsb: u16,
    pub source_texture_present: bool,
}

pub(super) fn scan_faces(
    tmd: &legaia_tmd::Tmd,
    bytes: &[u8],
    vram: &legaia_tim::Vram,
) -> Vec<Face> {
    let mut result = Vec::new();

    for (object_index, object) in tmd.objects.iter().enumerate() {
        let groups = legaia_tmd::legaia_prims::iter_groups_lenient(
            bytes,
            object.primitives_byte_offset,
            object.primitives_byte_size,
        );

        let mut ordinal = 0usize;

        for group in groups {
            for primitive in group.prims {
                let primitive_index = ordinal;
                ordinal += 1;

                if primitive.uvs.len() != 4 {
                    continue;
                }

                let indices = primitive.vertex_indices();
                if indices.len() != 4 {
                    continue;
                }

                let Some(vertices) = indices
                    .iter()
                    .map(|&id| object.vertices.get(id as usize))
                    .collect::<Option<Vec<_>>>()
                else {
                    continue;
                };

                let y = vertices[0].y;
                if vertices.iter().any(|v| v.y != y) {
                    continue;
                }

                let xmin = vertices.iter().map(|v| v.x).min().unwrap();
                let xmax = vertices.iter().map(|v| v.x).max().unwrap();
                let zmin = vertices.iter().map(|v| v.z).min().unwrap();
                let zmax = vertices.iter().map(|v| v.z).max().unwrap();

                if i32::from(xmax) - i32::from(xmin) != 128
                    || i32::from(zmax) - i32::from(zmin) != 128
                {
                    continue;
                }

                let actual = vertices
                    .iter()
                    .map(|v| (v.x, v.z))
                    .collect::<BTreeSet<_>>();

                let expected = [
                    (xmin, zmin),
                    (xmin, zmax),
                    (xmax, zmin),
                    (xmax, zmax),
                ];

                if !expected.iter().all(|p| actual.contains(p)) {
                    continue;
                }

                let umin = primitive.uvs.iter().map(|p| p.0).min().unwrap();
                let umax = primitive.uvs.iter().map(|p| p.0).max().unwrap();
                let vmin = primitive.uvs.iter().map(|p| p.1).min().unwrap();
                let vmax = primitive.uvs.iter().map(|p| p.1).max().unwrap();

                if umax - umin < 8 || vmax - vmin < 8 {
                    continue;
                }

                let corners = primitive.uvs
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>();

                if corners.len() != 4 {
                    continue;
                }

                let texture_present = vram.prim_has_texture_data(
                    primitive.cba,
                    primitive.tsb,
                    &primitive.uvs,
                );

                result.push(Face {
                    object: object_index,
                    primitive: primitive_index,
                    y,
                    cba: primitive.cba,
                    tsb: primitive.tsb,
                    source_texture_present: texture_present,
                });
            }
        }
    }

    result
}

/// Provisional source classification, not a claim that every
/// polygon in that scene is an interior or exterior floor.
fn category(source: &str) -> (&'static str, &'static str) {
    match source {
        "bylon" => ("Interior", "monastery masonry stone"),
        "retockin" => ("Interior", "room interior"),
        "rayman" | "rayman2" => ("Interior", "underground stone"),
        "jiji" => ("Interior", "cave nature stone"),
        "doman" => ("Interior", "research interior"),
        "koin1" | "koin2" | "koin3" | "koin4" =>
            ("Interior", "tower interior decorative"),
        "geremi" => ("Exterior", "town stone paving"),
        "town01" | "town0b" | "town0c" | "town0d" =>
            ("Exterior", "village ground nature"),
        _ => ("Unclassified", "original textured square"),
    }
}

fn matches_search(asset: &Asset, face: Face, query: &str) -> bool {
    let (area, style) = category(&asset.source);

    let identity = format!(
        "{} {} {} {} {} {} floor square flat \
         stone wood metal nature interior exterior \
         object {} primitive {} cba {:04x} tpage {:04x}",
        asset.source,
        asset.source_label,
        asset.label,
        asset.key,
        area,
        style,
        face.object,
        face.primitive,
        face.cba,
        face.tsb,
    ).to_lowercase();

    // Use actual style tags for material searches, not the
    // examples above as automatic matches for every tile.
    let searchable = identity.replace(
        "stone wood metal nature interior exterior",
        "",
    );

    query.split_whitespace().all(|word|
        searchable.contains(&word.to_lowercase())
    )
}

/// World Objects > Floor Tiles > Source Cell > Interior/Exterior
/// > Flat Squares > Original face.
pub(super) fn browser(
    ui: &mut egui::Ui,
    assets: &[Asset],
    search: &str,
) -> Option<(usize, Face)> {
    let mut indexed:
        BTreeMap<(String, String), Vec<(usize, Face)>> =
        BTreeMap::new();

    for (id, asset) in assets.iter().enumerate() {
        if !matches!(asset.kind, Kind::Mesh(_)) {
            continue;
        }

        for &face in &asset.floor_faces {
            if matches_search(asset, face, search) {
                indexed.entry((
                    asset.source_label.clone(),
                    asset.source.clone(),
                ))
                .or_default()
                .push((id, face));
            }
        }
    }

    let count: usize = indexed.values().map(Vec::len).sum();
    let mut choice = None;

    egui::CollapsingHeader::new(
        format!("Floor Tiles ({count} candidates)")
    )
    .id_salt("lck_c2_floors")
    .default_open(false)
    .show(ui, |ui| {
        ui.small(
            "Original TMD polygons, not fabricated textures. \
             Source classifications are provisional.",
        );

        for ((place, source), entries) in &indexed {
            egui::CollapsingHeader::new(
                format!("{} [{}] ({})", place, source, entries.len())
            )
            .id_salt(("lck_c2_floor_cell", source.as_str()))
            .default_open(false)
            .show(ui, |ui| {
                for area in ["Interior", "Exterior", "Unclassified"] {
                    let local: Vec<_> = entries.iter()
                        .filter(|(id, _)|
                            category(&assets[*id].source).0 == area
                        )
                        .collect();

                    if local.is_empty() {
                        continue;
                    }

                    egui::CollapsingHeader::new(
                        format!("{} ({})", area, local.len())
                    )
                    .id_salt(("lck_c2_floor_area", source.as_str(), area))
                    .default_open(false)
                    .show(ui, |ui| {
                        egui::CollapsingHeader::new(
                            format!("Flat Squares / 128 x 128 ({})", local.len())
                        )
                        .id_salt(("lck_c2_floor_type", source.as_str(), area))
                        .default_open(false)
                        .show(ui, |ui| {
                            // Avoid thousands of GUI rows in one frame.
                            for (id, face) in local.iter().take(150) {
                                let asset = &assets[*id];
                                let Kind::Mesh(slot) = asset.kind else {
                                    continue;
                                };

                                let texture = if face.source_texture_present {
                                    "Source texture data found"
                                } else {
                                    "Source texture unresolved"
                                };

                                let label = format!(
                                    "TMD {:03} / OBJ {} / Face {:03}{}",
                                    slot,
                                    face.object,
                                    face.primitive,
                                    if face.source_texture_present {
                                        ""
                                    } else {
                                        " [texture ?]"
                                    }
                                );

                                let tip = format!(
                                    "Original cell: {}\n\
                                     Face: TMD {} / object {} / primitive {}\n\
                                     TPAGE {:04X} / CLUT {:04X}\n\
                                     {}\n\
                                     Style: {}\n\
                                     Click for the actual source-model preview.\n\
                                     Native walkability authoring is not \
                                     yet implemented.",
                                    asset.source,
                                    slot,
                                    face.object,
                                    face.primitive,
                                    face.tsb,
                                    face.cba,
                                    texture,
                                    category(&asset.source).1,
                                );

                                if ui.selectable_label(false, label)
                                    .on_hover_text(tip)
                                    .clicked()
                                {
                                    choice = Some((*id, *face));
                                }
                            }
                            if local.len() > 150 {
                                ui.small(
                                    "Showing first 150. Use search to narrow.",
                                );
                            }
                        });
                    });
                }
            });
        }
    });

    choice
}

/// Isolate a true original TMD polygon, using its original UVs,
/// texture page, CLUT, PSX colours and vertex positions.
/// The existing donor VRAM supplies the actual texture pixels.
pub(super) fn preview_model(
    scene: &legaia_engine_scene::scene_assembly::AssembledScene,
    slot: usize,
    face: Face,
) -> Option<Model> {
    let resource = *scene.env_tmds.get(slot)?;
    let original = scene.res.tmds.get(resource)?;
    let object = original.tmd.objects.get(face.object)?;

    let primitive = legaia_tmd::legaia_prims::iter_groups_lenient(
        &original.raw,
        object.primitives_byte_offset,
        object.primitives_byte_size,
    )
    .into_iter()
    .flat_map(|g| g.prims)
    .nth(face.primitive)?;

    let indices = primitive.vertex_indices();
    if indices.len() != 4 || primitive.uvs.len() != 4 {
        return None;
    }

    let mut positions = Vec::with_capacity(4);

    for index in indices {
        let v = object.vertices.get(index as usize)?;
        positions.push([
            f32::from(v.x),
            -f32::from(v.y),
            f32::from(v.z),
        ]);
    }

    let mut model = Model {
        rest: positions.clone(),
        positions,
        object_ids: vec![0; 4],
        uvs: primitive.uvs
            .iter()
            .map(|&(u,v)| [u,v])
            .collect(),
        cba_tsb: vec![[primitive.cba, primitive.tsb]; 4],
        modulation: (0..4)
            .map(|i| primitive.colors.get(i)
                .or_else(|| primitive.colors.first())
                .copied()
                .unwrap_or([128;3]))
            .collect(),
        fill: vec![[180;3];4],
        textured: vec![1;4],
        indices: vec![0,1,2,1,3,2],
        center: [0.0;3],
        radius: 1.0,
    };

    model.refit();
    Some(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_classification_is_stable() {
        assert_eq!(category("bylon").0, "Interior");
        assert_eq!(category("geremi").0, "Exterior");
        assert_eq!(category("unknown").0, "Unclassified");
    }
}