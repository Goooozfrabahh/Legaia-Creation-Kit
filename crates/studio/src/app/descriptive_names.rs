//! Shared, owner-independent labels recovered from native object bindings.
//! Generated descriptions are defaults; user aliases and instance overrides win.
use std::collections::{BTreeMap, BTreeSet};
use legaia_engine_scene::{scene::{ProtIndex, Scene}, scene_assembly::AssembledScene};
use legaia_scene_formats::man_section;

/// Recognize explicit object nouns. Do not turn event labels or ambiguous
/// development abbreviations into claims about geometry or materials.
pub(super) fn description(native: &str) -> Option<&'static str> {
    const NOUNS: &[(&str, &str)] = &[
        ("通路ドア影", "Doorway shadow"), ("枯れ創世樹", "Withered Genesis tree"),
        ("創世樹", "Genesis tree"), ("家の中", "House interior"),
        ("家の壁", "House wall"), ("壁破片", "Wall debris"),
        ("壁左破壊", "Breakable wall"), ("壁右破壊", "Breakable wall"),
        ("壁外壁", "Exterior wall"), ("移動壁", "Moving wall"),
        ("壁火", "Wall flame"), ("通気管", "Ventilation pipe"),
        ("霧パイプ", "Mist pipe"), ("霧左パイプ", "Mist pipe"),
        ("霧マシーン", "Mist generator"), ("氷柱", "Ice pillar"),
        ("氷の台", "Ice platform"), ("雪ダマ", "Snowball"),
        ("赤色灯", "Red warning light"), ("鍵ケース", "Key case"),
        ("鍵穴", "Keyhole"), ("手紙の宝箱", "Treasure chest"),
        ("宝箱", "Treasure chest"), ("本棚", "Bookshelf"),
        ("食器棚", "Cupboard"), ("戸棚", "Cupboard"),
        ("たんす", "Dresser"), ("タンス", "Dresser"),
        ("引き出し", "Drawer"), ("植木鉢", "Plant pot"),
        ("植木", "Potted plant"), ("風車", "Windmill"),
        ("扇風機", "Fan"), ("暖炉", "Fireplace"),
        ("レバー", "Lever"), ("すいっち", "Switch"),
        ("スイッチ", "Switch"), ("手すり", "Railing"),
        ("鉄格子", "Metal bars"), ("水門", "Sluice gate"),
        ("牢屋", "Prison cell"), ("隔壁", "Partition wall"),
        ("エレドア", "Elevator doorway"), ("エアドア", "Airlock doorway"),
        ("ドア", "Doorway"), ("どあ", "Doorway"), ("扉", "Doorway"),
        ("えれべーた", "Elevator"), ("エレベーター", "Elevator"),
        ("エレべーター", "Elevator"), ("エレベタ", "Elevator"),
        ("エレベ", "Elevator"), ("エスカ", "Escalator"),
        ("レール", "Rail section"), ("階段", "Stairway"),
        ("支柱", "Support pillar"), ("移動柱", "Moving pillar"),
        ("揺れる柱", "Moving pillar"), ("ふさぎ岩", "Blocking rock"),
        ("崖肉", "Organic cliff face"), ("川岸", "Riverbank"),
        ("血管", "Blood vessel"), ("筒肉", "Organic tube"),
        ("穴管", "Pipe opening"), ("ちかかべ", "Underground wall"),
        ("ちかにく", "Underground organic scenery"),
        ("そらにく", "Overhead organic scenery"), ("わきにく", "Organic wall scenery"),
        ("血皮", "Organic surface"), ("炎血", "Flame effect"),
        ("プラント", "Plant"), ("食虫つた", "Carnivorous vine"),
        ("ベッド", "Bed"), ("テーブル", "Table"),
        ("料金表", "Price sign"), ("立て札", "Signpost"),
        ("看板", "Sign"), ("ポスター", "Poster"),
        ("カーテン", "Curtain"), ("マネキン", "Mannequin"),
        ("バルコニー", "Balcony"), ("ロープ", "Rope"),
        ("ステージ", "Stage platform"), ("ポンプ", "Pump"),
        ("ヤカン", "Kettle"), ("パイプ", "Pipe"),
        ("時計", "Clock"), ("紙切れ", "Paper"),
        ("温泉", "Hot spring"), ("プール", "Pool"),
        ("滝", "Waterfall"), ("海", "Sea surface"),
        ("夜空", "Night sky"), ("空色変え", "Sky"),
        ("僕の空", "Sky"), ("空だよーん", "Sky"),
        ("忌まわしき空", "Sky"), ("地雷な空", "Sky"),
        ("雲", "Cloud"), ("オーロラ", "Aurora effect"),
        ("けむり", "Smoke effect"), ("小火", "Flame effect"),
        ("炎", "Flame effect"), ("霧", "Mist effect"),
        ("ひかり", "Light effect"), ("スポット", "Spotlight"),
        ("橋", "Bridge"), ("ドーム", "Dome"), ("バリア", "Barrier"),
        ("ショップ２階", "Shop upper floor"), ("ショップ", "Shop building"),
        ("道具屋", "Shop building"), ("武器屋", "Shop building"),
        ("宿屋", "Inn building"), ("民家", "House"), ("の家", "House"),
        ("城本体", "Castle"), ("ガーメル城", "Castle"),
        ("回転道路", "Rotating walkway"), ("紋章", "Emblem"),
    ];
    for &(word, label) in NOUNS {
        if native.contains(word) { return Some(label); }
    }
    // Single-character nouns must match the whole label (apart from its
    // numeric variant), otherwise a person's name can become scenery.
    let stem = native.trim_end_matches(|c: char|
        c.is_ascii_digit() || ('０'..='９').contains(&c));
    match stem {
        "壁" | "壁Ａ" | "壁消え" => Some("Wall"),
        "木" => Some("Tree"), "草" | "草Ａ" => Some("Grass"),
        "山" | "山頂" => Some("Mountain section"),
        "氷" => Some("Ice section"), "水" => Some("Water surface"),
        "空" => Some("Sky"), "貝" => Some("Shell"),
        "花" | "はな" => Some("Flower"), "芽" => Some("Sprout"),
        "柵" => Some("Fence"), "板" => Some("Board"),
        "像Ａ" | "像Ｂ右" | "像Ｂ左" | "像Ｂ奥右" | "像Ｂ奥左" | "セルジュ像" => Some("Statue"),
        "泡" => Some("Bubble effect"), "光" | "光Ａ" => Some("Light effect"),
        "家" => Some("House"), "もん" | "げいと中" | "げいと外" => Some("Gate"),
        _ => None,
    }
}

fn native_object_name(man: &[u8], file: &man_section::ManFile, record: usize) -> Option<String> {
    let start = file.data_region_offset.checked_add(*file.partitions[0].get(record)? as usize)?;
    let count = usize::from(*man.get(start)?);
    if count == 0 || count > 80 { return None; }
    let end = start.checked_add(1 + count * 2)?;
    let next_record = file.partitions.iter().flatten()
        .map(|&offset| file.data_region_offset + offset as usize)
        .chain(file.sections.iter().map(|s| s.offset))
        .filter(|&offset| offset > start).min().unwrap_or(man.len());
    // A valid P0 header also has an animation byte after the name.
    if end >= next_record { return None; }
    let (text, _, invalid) = encoding_rs::SHIFT_JIS.decode(man.get(start + 1..end)?);
    let text = text.trim_matches('\0').trim();
    (!invalid && !text.is_empty() && text.chars().all(|c| !c.is_control()))
        .then(|| text.to_owned())
}

/// Return one default per visual identity, shared by placements, terrain and
/// reusable mesh rows. Native text is read through the actual scene adapters.
pub(super) fn for_scene(
    scene: &AssembledScene, index: &ProtIndex, visuals: &[Option<String>],
) -> BTreeMap<String, String> {
    let mut defaults = BTreeMap::new();
    for &resource in &scene.env_tmds {
        if let Some(key) = visuals.get(resource).and_then(Option::as_ref) {
            defaults.insert(key.clone(), "Scene model".to_owned());
        }
    }
    for (draws, label) in [(&scene.terrain, "Terrain section"), (&scene.placements, "World object")] {
        for draw in draws {
            if let Some(key) = visuals.get(draw.res_tmd).and_then(Option::as_ref) {
                defaults.insert(key.clone(), label.to_owned());
            }
        }
    }
    let Ok(native) = Scene::load(index, &scene.name) else { return defaults; };
    let Ok(Some(man)) = native.field_man_payload(index) else { return defaults; };
    let Ok(file) = man_section::parse(&man) else { return defaults; };
    let Ok((primary, fallback)) = native.field_tile_triggers(index) else { return defaults; };
    let labels: Vec<_> = (0..file.partitions[0].len())
        .map(|i| native_object_name(&man, &file, i).and_then(|s| description(&s)))
        .collect();
    let mut candidates: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for draw in &scene.placements {
        let Some(binding) = primary.iter().chain(&fallback)
            .find(|t| (t.tile_x, t.tile_z) == draw.anchor) else { continue; };
        let Some(label) = labels.get(binding.record as usize).copied().flatten() else { continue; };
        if let Some(key) = visuals.get(draw.res_tmd).and_then(Option::as_ref) {
            candidates.entry(key.clone()).or_default().insert(label);
        }
    }
    for (key, labels) in candidates {
        defaults.insert(key, labels.into_iter().collect::<Vec<_>>().join(" / "));
    }
    defaults
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptions_remove_owners_and_keep_explicit_object_details() {
        for (native, expected) in [
            ("主人公の家", "House"), ("友達の家", "House"),
            ("メイちゃん家の壁", "House wall"), ("主人公の家の中", "House interior"),
            ("女王ドア", "Doorway"), ("本棚女性寝室", "Bookshelf"),
            ("海４", "Sea surface"), ("壁破片左", "Wall debris"),
            ("枯れ創世樹", "Withered Genesis tree"), ("エレドア２", "Elevator doorway"),
        ] { assert_eq!(description(native), Some(expected), "{native}"); }
        for native in ["強", "高", "仮１", "Ｐ０２", "木ＩＮ", "木ＯＵＴ", "恋人ＩＮ"] {
            assert_eq!(description(native), None, "{native}");
        }
    }

    #[test]
    #[ignore = "Requires a local extracted retail disc via LCK_NAMING_SOURCE"]
    fn retail_repository_and_placements_share_descriptive_names() {
        let root = std::env::var("LCK_NAMING_SOURCE").expect("LCK_NAMING_SOURCE");
        let index = ProtIndex::open_extracted(std::path::Path::new(&root)).unwrap();
        let catalog = crate::asset_catalog::Catalog::default();
        for scene_name in ["town01", "bylon", "conc", "map01"] {
            let scene = legaia_engine_world_views::scene_staging::assemble_field_scene(&index, scene_name).unwrap();
            let world = super::super::world_view::build_world(&scene, &index);
            let assets = super::super::global_repository::catalog_one(&index, scene_name).unwrap();
            let mut named = 0;
            for asset in &assets {
                let resource = match asset.kind {
                    super::super::global_repository::Kind::Placement(i) => scene.placements[i].res_tmd,
                    super::super::global_repository::Kind::Terrain(i) => scene.terrain[i].res_tmd,
                    super::super::global_repository::Kind::Mesh(i) => scene.env_tmds[i],
                    _ => continue,
                };
                let key = asset.alias_key.as_ref().unwrap();
                let label = world.naming.descriptions.get(key).unwrap();
                assert_eq!(&asset.display_name(&catalog), label, "{scene_name} resource {resource}");
                if !matches!(label.as_str(), "World object" | "Terrain section" | "Scene model") { named += 1; }
            }
            assert!(named > 0, "No decoded descriptions in {scene_name}");
            eprintln!("{scene_name}: {named} repository rows have native descriptive names");
            if scene_name == "town01" {
                for label in ["Windmill", "Cupboard", "Clock", "Drawer", "House"] {
                    assert!(assets.iter().any(|a| a.label == label), "Missing {label}");
                }
                // Exercise the actual import, duplicate and serialized scene
                // restoration paths into a different destination cell.
                let slot = assets.iter().find_map(|asset| match asset.kind {
                    super::super::global_repository::Kind::Mesh(slot) if asset.label == "Windmill" => Some(slot),
                    _ => None,
                }).unwrap();
                let destination = legaia_engine_world_views::scene_staging::assemble_field_scene(&index, "bylon").unwrap();
                let mut edited = super::super::world_view::build_world(&destination, &index);
                let imported = edited.insert_foreign(&scene, "mesh", slot, 70001, [0.; 3], &index).unwrap();
                let copied = edited.duplicate_native(imported, destination.placements.len(), 70002, [128., 0., 0.]).unwrap();
                for key in [imported, copied] {
                    assert_eq!(edited.naming.descriptions.get(edited.base_name_key(key).unwrap()).unwrap(), "Windmill");
                }
                let text = super::super::scene_instances::serialize("bylon", &edited);
                let mut restored = super::super::world_view::build_world(&destination, &index);
                let document = super::super::scene_instances::parse(&text, "bylon", &restored).unwrap();
                super::super::scene_instances::apply(&document, &mut restored, destination.placements.len(), &index).unwrap();
                for uid in [70001, 70002] {
                    let draw = restored.draws.iter().find(|draw| draw.editor_uid == Some(uid)).unwrap();
                    let key = restored.base_name_key((draw.terrain, draw.index)).unwrap();
                    assert_eq!(restored.naming.descriptions.get(key).unwrap(), "Windmill");
                }
            }
        }
    }
}
