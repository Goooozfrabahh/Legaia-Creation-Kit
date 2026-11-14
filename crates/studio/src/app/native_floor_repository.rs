//! R623 - Original MAP floor textures, indexed by native source identity.
//! No TMD polygon classification. No manufactured material names.

use std::collections::BTreeMap;
use serde::{Serialize,Deserialize};
use eframe::egui;
use super::{
    App,Viewport,Shading,
    repo_jump,global_repository,favorites
};

#[derive(Clone,Debug,PartialEq,Eq,PartialOrd,Ord,Hash,Serialize,Deserialize)]
pub(super) struct Key {
    pub source:String,
    pub atlas:u8,
    pub tpage:u16,
    pub clut:u16,
    pub uvs:[[u8;2];4],
}

#[derive(Clone,Debug)]
pub(super) struct Tile {
    pub key:Key,
    pub source_label:String,
    pub sample:[u8;2],
    pub cells:Vec<[u8;2]>,
    pub flat_count:usize,
    pub textured:bool,
}

pub(super) fn index_scene(
    scene:&legaia_engine_scene::scene_assembly::AssembledScene
)->Vec<Tile>{
    let Some(hf)=scene.ground.as_ref() else{return Vec::new()};
    let mut indexed:BTreeMap<Key,Tile>=BTreeMap::new();

    for (q,points) in hf.positions.chunks_exact(4).enumerate(){
        if q>=hf.quad_count(){break;}
        let base=q*4;

        let x=(points[0][0]/128.0).round() as i32;
        let z=(points[0][2]/128.0).round() as i32;

        if !(0..128).contains(&x) || !(0..128).contains(&z){
            continue;
        }

        let Some(&atlas)=hf.tile_ids.get(base) else{continue};
        let Some(&page)=hf.cba_tsb.get(base) else{continue};
        let Some(uv)=hf.uvs.get(base..base+4) else{continue};
        if hf.cba_tsb.get(base..base+4)
            .is_none_or(|p|p.iter().any(|v|*v!=page)){
            continue;
        }

        let uvs=[uv[0],uv[1],uv[2],uv[3]];
        let cell=[x as u8,z as u8];
        let flat=points.iter()
            .all(|p|(p[1]-points[0][1]).abs()<0.5);

        let key=Key{
            source:scene.name.clone(),
            atlas,
            tpage:page[1],
            clut:page[0],
            uvs,
        };

        if let Some(tile)=indexed.get_mut(&key){
            tile.cells.push(cell);
            if flat {
                if tile.flat_count==0{tile.sample=cell;}
                tile.flat_count+=1;
            }
            continue;
        }

        let uv_pairs=uvs.map(|[u,v]|(u,v));
        let textured=scene.res.vram.prim_has_texture_data(
            page[0],page[1],&uv_pairs
        );

        indexed.insert(key.clone(),Tile{
            key,
            source_label:crate::place_name(&scene.name)
                .unwrap_or(scene.name.as_str()).to_owned(),
            sample:cell,
            cells:vec![cell],
            flat_count:usize::from(flat),
            textured,
        });
    }

    indexed.into_values().collect()
}

#[derive(Clone)]
pub(super) enum Action {
    Preview(Key),
    ToggleFavorite(Key),
    Drag(Key),
    Place(Key),
}

fn label(tile:&Tile)->String{
    format!(
        "Atlas {:02} | TPAGE {:04X} | CLUT {:04X} | {} cells{}",
        tile.key.atlas,
        tile.key.tpage,
        tile.key.clut,
        tile.cells.len(),
        if tile.textured {""} else {" [texture unresolved]"}
    )
}

pub(super) fn browser(
    ui:&mut egui::Ui,
    tiles:&[Tile],
    favorites:&favorites::Store,
    search:&str,
    selected:Option<&Key>,
    reveal:Option<&Key>,
)->(Option<Action>,bool){
    let mut action=None;
    let mut scrolled=false;
    let term=search.trim().to_lowercase();

    let mut families:
        BTreeMap<String,BTreeMap<String,Vec<&Tile>>>=
        BTreeMap::new();

    for tile in tiles{
        let searchable=format!(
            "{} {} {} atlas {} tpage {:04x} clut {:04x} {}",
            tile.key.source,
            tile.source_label,
            label(tile),
            tile.key.atlas,
            tile.key.tpage,
            tile.key.clut,
            tile.key.source
        ).to_lowercase();

        if !term.is_empty() &&
            !term.split_whitespace().all(|w|searchable.contains(w)) &&
            selected!=Some(&tile.key){
            continue;
        }

        let family=super::floor_tiles::floor_master(&tile.key.source)
            .unwrap_or(tile.source_label.as_str()).to_owned();

        families.entry(family).or_default()
            .entry(tile.key.source.clone()).or_default()
            .push(tile);
    }

    egui::CollapsingHeader::new(
        format!("Floor Tiles - Native MAP ({} variants)",tiles.len())
    )
    .id_salt("r623_native_map_floors")
    .default_open(true)
    .open(selected.is_some().then_some(true))
    .show(ui,|ui|{
        ui.small(
            "Actual 128x128 MAP ground textures. Entries are distinct \
             atlas/TPAGE/CLUT/UV combinations, grouped by original scene variant."
        );

        for (family,variants) in &families{
            let in_family=selected.is_some_and(|key|
                variants.contains_key(&key.source)
            );

            egui::CollapsingHeader::new(format!(
                "{family} ({} variants)",variants.len()
            ))
            .id_salt(("r623_floor_family",family))
            .default_open(false)
            .open(in_family.then_some(true))
            .show(ui,|ui|{
                for (source,rows) in variants{
                    let in_variant=selected.is_some_and(
                        |key|key.source==*source
                    );

                    egui::CollapsingHeader::new(format!(
                        "{} [{}] - 128x128 ({})",
                        rows[0].source_label,
                        source,
                        rows.len()
                    ))
                    .id_salt(("r623_floor_variant",source))
                    .default_open(false)
                    .open(in_variant.then_some(true))
                    .show(ui,|ui|{
                        for tile in rows{
                            let favorite=favorites::Favorite::native(tile);
                            let favored=favorites.has(&favorite);

                            let caption=format!(
                                "{}{}",
                                if favored {"★ "}else{""},
                                label(tile)
                            );

                            let response=ui.add(
                                egui::Button::new(caption)
                                    .selected(selected==Some(&tile.key))
                                    .sense(egui::Sense::click_and_drag())
                            ).on_hover_text(format!(
                                "Original source: {}\n\
                                 Example cell: [{},{}]\n\
                                 Flat examples: {}\n\
                                 Uses: {}\n\
                                 TPAGE {:04X} / CLUT {:04X}\n\
                                 Native MAP texture; no inferred material name",
                                tile.key.source,
                                tile.sample[0],tile.sample[1],
                                tile.flat_count,tile.cells.len(),
                                tile.key.tpage,tile.key.clut
                            ));

                            if response.clicked(){
                                action=Some(Action::Preview(tile.key.clone()));
                            }
                            if response.drag_started(){
                                action=Some(Action::Drag(tile.key.clone()));
                            }

                            if reveal==Some(&tile.key){
                                response.scroll_to_me(
                                    Some(egui::Align::Center)
                                );
                                scrolled=true;
                            }

                            response.context_menu(|ui|{
                                if ui.button("Preview original texture").clicked(){
                                    action=Some(Action::Preview(tile.key.clone()));
                                    ui.close();
                                }
                                if ui.button(if favored{
                                    "Remove from Favorites"
                                }else{
                                    "Add to Favorites"
                                }).clicked(){
                                    action=Some(Action::ToggleFavorite(
                                        tile.key.clone()
                                    ));
                                    ui.close();
                                }
                                if ui.button("Duplicate in original cell").clicked(){
                                    action=Some(Action::Place(tile.key.clone()));
                                    ui.close();
                                }
                                ui.separator();
                                ui.small(format!(
                                    "Original MAP source: {} / atlas {}",
                                    source,tile.key.atlas
                                ));
                            });
                        }
                    });
                }
            });
        }
    });

    (action,scrolled)
}

impl App{
    pub(super) fn r623_open_floor(
        &mut self,key:Key
    )->Result<(),String>{
        let tile=self.native_floor_tiles.iter()
            .find(|tile|tile.key==key)
            .cloned()
            .ok_or("Native MAP floor resource not indexed")?;

        let lib=self.studio.library.as_ref()
            .ok_or("Open the original disc IMG first")?;

        let scene=legaia_engine_world_views::scene_staging::
            assemble_field_scene(&lib.index,&key.source)
            .map_err(|e|format!(
                "Cannot assemble original floor source {}: {e}",
                key.source
            ))?;

        let (model,ground_metadata)=repo_jump::ground_quad(
            &scene,tile.sample
        ).ok_or("Native ground quad could not be decoded")?;

        self.native_floor_selected=Some(key.clone());
        self.native_floor_reveal=Some(key.clone());
        self.global_repository_dialog=None;

        self.repo_jump_native=Some(repo_jump::SourcePreview{
            source:key.source.clone(),
            kind:"Native MAP ground".into(),
            title:format!(
                "Atlas {} - {}",
                key.atlas,tile.source_label
            ),
            details:format!(
                "Original 128x128 MAP floor texture\n\
                 TPAGE {:04X} / CLUT {:04X}\n\
                 {} original cell use(s)\n\
                 Example: [{},{}]\n{}",
                key.tpage,key.clut,
                tile.cells.len(),
                tile.sample[0],tile.sample[1],
                ground_metadata
            ),
            scene:Some(scene),
            model:Some(model),
        });

        self.repo_jump_view=Viewport::new(Shading::Textured);
        self.repo_jump_reveal=false;
        Ok(())
    }
}

#[cfg(test)]
mod tests{
    use super::*;

    #[test]
    fn variants_and_palettes_have_distinct_identity(){
        let a=Key{
            source:"conc".into(),
            atlas:42,
            tpage:0x000c,
            clut:0x7f46,
            uvs:[[0,0],[31,0],[0,31],[31,31]],
        };
        let mut b=a.clone();
        b.clut=0x7f47;
        assert_ne!(a,b);
        b=a.clone();
        b.source="conc2".into();
        assert_ne!(a,b);
    }
}