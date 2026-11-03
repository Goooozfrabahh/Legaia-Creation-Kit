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
    pub shape: &'static str,
    pub span_x: i32,
    pub span_z: i32,
    pub rise: i32,
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

                if !matches!(primitive.uvs.len(), 3 | 4) {
                    continue;
                }

                let indices = primitive.vertex_indices();
                if indices.len() != primitive.uvs.len() {
                    continue;
                }

                let Some(vertices) = indices
                    .iter()
                    .map(|&id| object.vertices.get(id as usize))
                    .collect::<Option<Vec<_>>>()
                else {
                    continue;
                };

                // LCK_C2_EXPANDED_TMD_FLOORS_V1
                // Preserve all textured original triangles/quads.
                let y = vertices[0].y;
                let min_x = vertices.iter().map(|v| i32::from(v.x)).min().unwrap();
                let max_x = vertices.iter().map(|v| i32::from(v.x)).max().unwrap();
                let min_z = vertices.iter().map(|v| i32::from(v.z)).min().unwrap();
                let max_z = vertices.iter().map(|v| i32::from(v.z)).max().unwrap();
                let min_y = vertices.iter().map(|v| i32::from(v.y)).min().unwrap();
                let max_y = vertices.iter().map(|v| i32::from(v.y)).max().unwrap();

                let span_x = max_x - min_x;
                let span_z = max_z - min_z;
                let rise = max_y - min_y;

                let square_corners = vertices.len() == 4 &&
                    [(min_x,min_z),(min_x,max_z),(max_x,min_z),(max_x,max_z)]
                        .iter().all(|&(x,z)| vertices.iter()
                            .any(|v| i32::from(v.x)==x && i32::from(v.z)==z));

                let shape = if span_x == 0 || span_z == 0 {
                    "Vertical / Degenerate / Other"
                } else if rise > 0 {
                    "Slopes / Uneven Surfaces"
                } else if vertices.len() == 3 {
                    "Flat Triangles"
                } else if square_corners && span_x == 128 && span_z == 128 {
                    "Flat Squares / 128 x 128"
                } else if square_corners && span_x == span_z {
                    "Flat Squares / Other Sizes"
                } else if square_corners {
                    "Flat Rectangles / Strips"
                } else {
                    "Flat Irregular Quads"
                };

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
                    shape,
                    span_x,
                    span_z,
                    rise,
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
    if query.trim().is_empty() { return true; }
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
    let searchable = format!("{} {} {} {} {}",
        identity.replace("stone wood metal nature interior exterior", "")
            .replace("floor square flat", "floor candidate"),
        face.shape, face.span_x, face.span_z, face.rise);

    query.split_whitespace().all(|word|
        searchable.contains(&word.to_lowercase())
    )
}


/// Step through the same floor candidates shown in one source-cell
/// Flat Squares list. Wrap around at either end.
pub(super) fn adjacent(
    assets: &[Asset],
    search: &str,
    current: (usize, Face),
    step: i32,
) -> Option<(usize, Face)> {
    let source = assets.get(current.0)?.source.as_str();
    let mut rows = Vec::new();

    for (id, asset) in assets.iter().enumerate() {
        if asset.source.as_str() != source ||
           !matches!(asset.kind, Kind::Mesh(_)) {
            continue;
        }

        for &face in &asset.floor_faces {
            if matches_search(asset, face, search) {
                rows.push((id, face));
                if rows.len() >= 150 {
                    break;
                }
            }
        }

        if rows.len() >= 150 {
            break;
        }
    }

    let index = rows.iter().position(|&row| row == current)?;
    let next = if step > 0 {
        (index + 1) % rows.len()
    } else if step < 0 {
        (index + rows.len() - 1) % rows.len()
    } else {
        index
    };

    rows.get(next).copied()
}

/// World Objects > Floor Tiles > Source Cell > Interior/Exterior
/// > Flat Squares > Original face.

// LCK_C2_FLOOR_MASTER_DRAG_V3
// Group only verified original scene-family IDs.
fn floor_master(source:&str)->Option<&'static str> {
    match source {
        "town01"|"town0b"|"town0c"|"town0d"|"town0e"|
        "jou"|"urudre1" => Some("Rim Elm"),

        "conc"|"concnow"|"conc2"|"conc3"|"concend"|
        "urudre2"|"opurud" => Some("Conkram"),

        "rayman"|"rayman2"|"ropeway"|"ropeway2"|"station"
            => Some("Octam"),

        "balden"|"balden2"|"edbalden" => Some("Vidna"),
        "bubu1"|"bubu2"|"edbubu" => Some("Buma"),
        "retock"|"retockin"|"edretoin" => Some("Ratayu"),

        "bylon"|"urudre3"|"edbylon"
            => Some("Biron Monastery"),

        "dolk"|"dolk2" => Some("Drake Castle"),
        "taiku"|"taiku2" => Some("Zora's Floating Castle"),
        "nilboa"|"nilboa2" => Some("Nivora Ravine"),
        "rikuroa"|"rikuroa2" => Some("Mt. Rikuroa"),
        "uru"|"uru2" => Some("Uru Mais"),
        "deroa"|"chitei2" => Some("Jette's Fortress"),
        "teien"|"tower"|"edteien" => Some("Sky Gardens"),

        "kor"|"kor3"|"kor4"|"kor5"|"korb2"|"korb3"|
        "korout"|"koin1"|"koin1b"|"koin2"|"koin3"|
        "koin4"|"koin6"|"edkorout" => Some("Sol Tower"),

        "juui1"|"juui2"|"jouina"|"jouinb"|"jouinc"|
        "jouind"|"jouine" => Some("Bio Castle"),

        "station3"|"opstati"|"edstati3"
            => Some("Karisto Station"),

        "doman"|"eddoman" => Some("Usha Research Center"),
        "son"|"edson" => Some("Soren Camp"),
        "deene"|"opdeene" => Some("Mt. Dhini"),
        "tunnelb"|"tunnelc" => Some("Fire Path"),
        _ => None,
    }
}

pub(super) fn browser(
    ui: &mut egui::Ui,
    assets: &[Asset],
    search: &str,
    selected: Option<(usize, Face)>,
    scroll_target: Option<(usize, Face)>,
) -> (Option<(usize, Face)>, bool) {
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
    let mut scrolled = false;

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

        let mut families: BTreeMap<String,Vec<_>> = BTreeMap::new();

        for (key,entries) in &indexed {
            let family=floor_master(key.1.as_str())
                .unwrap_or(key.0.as_str()).to_owned();
            families.entry(family).or_default().push((key,entries));
        }

        for (family,variants) in &families {
            let mut render_variants=|ui: &mut egui::Ui| {
                for (key,entries) in variants {
                    let place=&key.0;
                    let source=&key.1;
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
                        for shape in [
                            "Flat Squares / 128 x 128",
                            "Flat Squares / Other Sizes",
                            "Flat Rectangles / Strips",
                            "Flat Triangles",
                            "Flat Irregular Quads",
                            "Slopes / Uneven Surfaces",
                            "Vertical / Degenerate / Other",
                        ] {
                            let bucket: Vec<_> = local.iter().copied()
                                .filter(|(_,face)| face.shape == shape).collect();
                            if bucket.is_empty() {continue;}

                        egui::CollapsingHeader::new(
                            format!("{} ({})", shape, bucket.len())
                        )
                        .id_salt(("lck_c2_floor_type", source.as_str(), area, shape))
                        .default_open(false)
                        .show(ui, |ui| {
                            // Avoid thousands of GUI rows in one frame.
                            for (id, face) in bucket.iter() {
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
                                    "{}x{} rise {} | TMD {:03} / OBJ {} / Face {:03}{}",
                                    face.span_x, face.span_z, face.rise,
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

                                let row = (*id, *face);
                                let response = ui.add(
                                    egui::Button::new(label)
                                        .selected(selected == Some(row))
                                        .sense(egui::Sense::click_and_drag())
                                ).on_hover_text(tip);

                                // Actual per-face drag, not parent TMD drag.
                                if response.drag_started() ||
                                    ui.input(|i| i.pointer.primary_pressed() &&
                                        i.pointer.interact_pos().is_some_and(
                                            |p| response.rect.contains(p)
                                        )
                                    ) {
                                    ui.ctx().data_mut(|data| {
                                        data.insert_temp(
                                            egui::Id::new("lck_c2_floor_drag_face"),
                                            row
                                        );
                                    });
                                }

                                if scroll_target == Some(row) {
                                    response.scroll_to_me(
                                        Some(egui::Align::Center)
                                    );
                                    scrolled = true;
                                }

                                if response.clicked() {
                                    choice = Some(row);
                                }
                            }

                        });
                        }
                    });
                }
            });
                }
            };

            let known_family = variants.iter().any(|(key,_)|
                floor_master(key.1.as_str()).is_some()
            );

            if known_family {
                egui::CollapsingHeader::new(
                    format!("{} ({} variants)",family,variants.len())
                )
                .id_salt(("lck_c2_floor_master",family.as_str()))
                .default_open(false)
                .show(ui,|ui| render_variants(ui));
            } else {
                render_variants(ui);
            }
        }
    });

    (choice, scrolled)
}


/// Original per-face donor identity, retained across scene saves.
pub(super) fn encode_floor_kind(face: Face) -> String {
    format!("floor_{}_{}", face.object, face.primitive)
}

pub(super) fn decode_floor_kind(kind: &str) -> Option<(usize,usize)> {
    let encoded = kind.strip_prefix("floor_")?;
    let (object, primitive) = encoded.split_once('_')?;
    if primitive.contains('_') { return None; }
    let object = object.parse::<usize>().ok()?;
    let primitive = primitive.parse::<usize>().ok()?;
    (object < 65536 && primitive < 65536).then_some((object,primitive))
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
    if !matches!(indices.len(), 3 | 4) || indices.len() != primitive.uvs.len() {
        return None;
    }

    let count = indices.len();
    let mut positions = Vec::with_capacity(count);

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
        object_ids: vec![0; count],
        uvs: primitive.uvs
            .iter()
            .map(|&(u,v)| [u,v])
            .collect(),
        cba_tsb: vec![[primitive.cba, primitive.tsb]; count],
        modulation: (0..count)
            .map(|i| primitive.colors.get(i)
                .or_else(|| primitive.colors.first())
                .copied()
                .unwrap_or([128;3]))
            .collect(),
        fill: vec![[180;3];count],
        textured: vec![1;count],
        indices: if count == 3 {vec![0,1,2]} else {vec![0,1,2,1,3,2]},
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
    fn arrows_follow_source_list_and_wrap() {
        let make_face = |n| Face { shape: "Flat Squares / 128 x 128", span_x: 128, span_z: 128, rise: 0,
            object: 0,
            primitive: n,
            y: 0,
            cba: 0,
            tsb: 0,
            source_texture_present: true,
        };

        let make_asset = |source: &str, slot: usize, faces: Vec<Face>| Asset {
            source: source.into(),
            source_label: source.into(),
            label: "Original TMD".into(),
            key: format!("cell:{source}|mesh:{slot}"),
            alias_key: None,
            kind: Kind::Mesh(slot),
            mesh_fingerprint: Some(slot as u64),
            floor_faces: faces,
        };

        let rows = vec![
            make_asset("bylon", 0, vec![make_face(1), make_face(2)]),
            make_asset("bylon", 1, vec![make_face(3)]),
            make_asset("geremi", 0, vec![make_face(4)]),
        ];

        assert_eq!(
            adjacent(&rows, "", (0, make_face(1)), 1),
            Some((0, make_face(2)))
        );
        assert_eq!(
            adjacent(&rows, "", (0, make_face(2)), 1),
            Some((1, make_face(3)))
        );
        assert_eq!(
            adjacent(&rows, "", (1, make_face(3)), 1),
            Some((0, make_face(1)))
        );
        assert_eq!(
            adjacent(&rows, "", (0, make_face(1)), -1),
            Some((1, make_face(3)))
        );
        assert_eq!(
            adjacent(&rows, "no-such-texture", (0, make_face(1)), 1),
            None
        );
    }

    #[test]
    fn source_classification_is_stable() {
        assert_eq!(category("bylon").0, "Interior");
        assert_eq!(category("geremi").0, "Exterior");
        assert_eq!(category("unknown").0, "Unclassified");
    }
}