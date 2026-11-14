// Native walk-ground selection, read-only.
use std::collections::BTreeSet;
use eframe::egui::{self, Color32, Stroke};
use legaia_scene_formats::field_objects::WalkHeightfield;
use crate::raster::{Camera, Model};
use super::{collision_wire::Projector, viewport::Viewport};

type Cell = [u8; 2];

fn cell_at(hf: &WalkHeightfield, quad: usize) -> Option<Cell> {
    let p=hf.positions.get(quad.checked_mul(4)?)?;
    let x=(p[0]/128.0).round() as i32;
    let z=(p[2]/128.0).round() as i32;
    if !(0..128).contains(&x) || !(0..128).contains(&z) {
        return None;
    }
    Some([x as u8,z as u8])
}

pub(super) fn pick_cell(
    hf:&WalkHeightfield,model:&Model,cam:Camera,
    w:usize,h:usize,pixel:[f32;2]
)->Option<Cell> {
    if w==0 || h==0 || hf.indices.is_empty() {return None;}
    let (sy,cy)=cam.yaw.sin_cos();
    let (sp,cp)=cam.pitch.sin_cos();
    let fov=45f32.to_radians();
    let f=(h as f32*0.5)/(fov*0.5).tan();
    let dist=model.radius/(fov*0.5).sin()
        *cam.zoom.max(0.0000001);
    let near=(dist*0.003).max(0.0001);

    let projected:Vec<[f32;3]>=model.positions.iter().map(|p|{
        let x=p[0]-(model.center[0]+cam.target_offset[0]);
        let y=p[1]-(model.center[1]+cam.target_offset[1]);
        let z=p[2]-(model.center[2]+cam.target_offset[2]);
        let x1=x*cy+z*sy;
        let z1=-x*sy+z*cy;
        let y2=y*cp-z1*sp;
        let z2=y*sp+z1*cp+dist;
        [
            w as f32*0.5+f*x1/z2,
            h as f32*0.5+f*y2/z2,
            z2
        ]
    }).collect();

    let mut nearest=f32::INFINITY;
    let mut winner=None;
    let [px,py]=pixel;
    let ground_triangles=hf.quad_count()*2;

    // All geometry participates in depth testing. A building
    // should block selection of ground behind that building.
    for (tri_no,tri) in model.indices.chunks_exact(3).enumerate() {
        let i=[tri[0] as usize,tri[1] as usize,tri[2] as usize];
        if i.iter().any(|&k|k>=projected.len()) {continue;}
        let s=[projected[i[0]],projected[i[1]],projected[i[2]]];
        if s.iter().any(|p| !p[2].is_finite()||p[2]<near) {
            continue;
        }
        let area=(s[1][0]-s[0][0])*(s[2][1]-s[0][1])-
            (s[2][0]-s[0][0])*(s[1][1]-s[0][1]);
        if area.abs()<1e-6 {continue;}

        let w0=((s[1][0]-px)*(s[2][1]-py)-
            (s[2][0]-px)*(s[1][1]-py))/area;
        let w1=((s[2][0]-px)*(s[0][1]-py)-
            (s[0][0]-px)*(s[2][1]-py))/area;
        let w2=1.0-w0-w1;
        if w0<0.0||w1<0.0||w2<0.0 {continue;}

        let z=w0*s[0][2]+w1*s[1][2]+w2*s[2][2];
        if z<nearest {
            nearest=z;
            winner=if tri_no<ground_triangles {
                Some(tri_no/2)
            } else {None};
        }
    }
    winner.and_then(|quad|cell_at(hf,quad))
}

pub(super) fn apply(
    selected:&mut BTreeSet<Cell>,
    active:&mut Option<Cell>,
    clicked:Option<Cell>,shift:bool,ctrl:bool
){
    if !shift&&!ctrl {selected.clear();}
    if let Some(cell)=clicked {
        if ctrl {
            if !selected.remove(&cell) {selected.insert(cell);}
        } else {
            selected.insert(cell);
        }
        *active=if selected.contains(&cell) {
            Some(cell)
        } else {
            selected.iter().next_back().copied()
        };
    } else if !shift&&!ctrl {
        *active=None;
    }
}

pub(super) fn paint(
    ui:&egui::Ui,rect:egui::Rect,view:&Viewport,model:&Model,
    hf:&WalkHeightfield,selected:&BTreeSet<Cell>
){
    if selected.is_empty() {return;}
    let projection=Projector::new(ui,rect,view,model);
    let painter=ui.painter_at(rect);
    let color=Color32::from_rgb(255,197,58);

    for (quad,points) in hf.positions.chunks_exact(4).enumerate() {
        if !cell_at(hf,quad).is_some_and(
            |cell|selected.contains(&cell)
        ) {continue;}

        let vertices=points.iter()
            .map(|p|projection.project([p[0],-p[1],p[2]]))
            .collect::<Option<Vec<_>>>();
        let Some(v)=vertices else {continue;};

        for (a,b) in [(0,1),(1,3),(3,2),(2,0)] {
            painter.line_segment(
                [v[a],v[b]],Stroke::new(2.5,color)
            );
        }
    }
}

pub(super) fn inspector(
    ui:&mut egui::Ui,hf:&WalkHeightfield,active:Option<Cell>
){
    ui.small("Click: select | Shift: add | Ctrl: toggle | Drag empty ground: pan.");
    ui.small("Selection only. Walkability, heights, and original MAP bytes remain unchanged.");

    let Some(cell)=active else {return;};
    for (quad,points) in hf.positions.chunks_exact(4).enumerate() {
        if cell_at(hf,quad)!=Some(cell) {continue;}
        let base=quad*4;
        let id=hf.tile_ids.get(base).copied().unwrap_or(0);
        let [clut,tpage]=hf.cba_tsb.get(base)
            .copied().unwrap_or([0,0]);
        let tiers=hf.corner_tiers.get(base..base+4)
            .unwrap_or(&[]);
        let heights=points.iter()
            .map(|p|format!("{:.0}",-p[1]))
            .collect::<Vec<_>>().join(", ");

        ui.monospace(format!(
            "MAP cell ({},{}) | World X {}..{}, Z {}..{}",
            cell[0],cell[1],
            cell[0] as u32*128,(cell[0] as u32+1)*128,
            cell[1] as u32*128,(cell[1] as u32+1)*128
        ));
        ui.monospace(format!(
            "Atlas tile {} | TPAGE {:04X} | CLUT {:04X}",
            id,tpage,clut
        ));
        ui.monospace(format!(
            "Corner tiers {:?} | Heights (Y up) [{}]",
            tiers,heights
        ));
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ground_selection_single_add_toggle_and_clear(){
        let mut set=BTreeSet::new();
        let mut active=None;
        apply(&mut set,&mut active,Some([4,7]),false,false);
        assert_eq!(set.len(),1);
        apply(&mut set,&mut active,Some([5,7]),true,false);
        assert_eq!(set.len(),2);
        apply(&mut set,&mut active,Some([4,7]),false,true);
        assert_eq!(set.len(),1);
        assert_eq!(active,Some([5,7]));
        apply(&mut set,&mut active,None,false,false);
        assert!(set.is_empty());
        assert_eq!(active,None);
    }
}


// LCK_GROUND_SCENE_HIERARCHY_V1
// Read-only hierarchy references to original native MAP ground cells.
// One entry per actual rendered 128x128 cell, never per source TMD face.
pub(super) fn hierarchy(
    ui: &mut egui::Ui,
    hf: &WalkHeightfield,
    selected: &mut BTreeSet<Cell>,
    active: &mut Option<Cell>,
    reveal: &mut Option<Cell>,
    filter: &str,
    edits: &mut super::ground_edit_ops::State,
) -> bool {
    use std::collections::BTreeMap;

    // Sixteen-row regions avoid opening thousands of widgets at once.
    // Region -> Z row -> [original cell, atlas ID, texture page].
    let mut regions:
        BTreeMap<u8, BTreeMap<u8, Vec<(Cell,u8,u16)>>> =
        BTreeMap::new();

    let term = filter.trim().to_lowercase();

    for (quad,_) in hf.positions.chunks_exact(4).enumerate() {
        let Some(cell)=cell_at(hf,quad) else {continue;};
        if edits.deleted.contains(&cell){continue;}
        let vertex=quad*4;
        let atlas=hf.tile_ids.get(vertex).copied().unwrap_or(0);
        let tpage=hf.cba_tsb.get(vertex)
            .map(|pair|pair[1]).unwrap_or(0);

        let label=format!(
            "Tile [{:03}, {:03}]  Atlas {:02}  TPAGE {:04X}",
            cell[0],cell[1],atlas,tpage
        );

        if !term.is_empty()
            && !label.to_lowercase().contains(&term)
            && !selected.contains(&cell)
            && *reveal != Some(cell)
        {
            continue;
        }

        regions.entry(cell[1]/16)
            .or_default()
            .entry(cell[1])
            .or_default()
            .push((cell,atlas,tpage));
    }

    let count=hf.positions.len()/4;
    let focus=(*active).or(*reveal);
    let mut changed=false;

    egui::CollapsingHeader::new(format!("Floor Tiles ({count})"))
        .id_salt("lck_native_ground_hierarchy")
        .default_open(false)
        .open((focus.is_some() || edits.active_copy.is_some() || !term.is_empty()).then_some(true))
        .show(ui,|ui| {
            ui.small("Original native MAP ground cells");
            if !edits.deleted.is_empty() {
                egui::CollapsingHeader::new(format!(
                    "Deleted native tiles ({})",edits.deleted.len()
                ))
                .id_salt("g3_deleted_native_tiles")
                .show(ui,|ui|{
                    for cell in edits.deleted.iter().copied().collect::<Vec<_>>() {
                        if ui.button(format!(
                            "Restore tile [{}, {}]",cell[0],cell[1]
                        )).clicked() {
                            edits.deleted.remove(&cell);
                            changed=true;
                        }
                    }
                });
            }
            changed |= super::ground_edit_ops::copies_hierarchy(ui,edits,selected,active,&term);

            for (region,rows) in &regions {
                let start=*region as usize*16;
                let end=start+15;
                let amount:usize=rows.values().map(|r|r.len()).sum();

                let focus_region=focus.is_some_and(
                    |cell|cell[1]/16==*region
                );

                egui::CollapsingHeader::new(format!(
                    "Z Rows {start}-{end} ({amount})"
                ))
                    .id_salt(("lck_ground_region",*region))
                    .default_open(false)
                    .open((focus_region || !term.is_empty()).then_some(true))
                    .show(ui,|ui| {
                        for (row,tiles) in rows {
                            let focus_row=focus.is_some_and(
                                |cell|cell[1]==*row
                            );

                            egui::CollapsingHeader::new(format!(
                                "Row Z={row} ({})",tiles.len()
                            ))
                                .id_salt(("lck_ground_row",*row))
                                .default_open(false)
                                .open((focus_row || !term.is_empty()).then_some(true))
                                .show(ui,|ui| {
                                    for &(cell,atlas,tpage) in tiles {
                                        let is_selected=selected.contains(&cell);
                                        let response=ui.selectable_label(
                                            is_selected,
                                            format!(
                                                "Tile [{:03}, {:03}]  Atlas {:02}  TPAGE {:04X}",
                                                cell[0],cell[1],atlas,tpage
                                            )
                                        ).on_hover_text(format!(
                                            "Original MAP ground cell\n\
                                             Grid X={}, Z={}\n\
                                             World X={}..{}, Z={}..{}\n\
                                             Atlas tile {}\n\
                                             TPAGE {:04X}\n\
                                             Click to highlight this ground cell",
                                            cell[0],cell[1],
                                            cell[0] as u32*128,
                                            (cell[0] as u32+1)*128,
                                            cell[1] as u32*128,
                                            (cell[1] as u32+1)*128,
                                            atlas,tpage
                                        ));

                                        if response.clicked() {
                                            let modifiers=ui.input(|i|i.modifiers);
                                            if !modifiers.shift&&!modifiers.ctrl {
                                                edits.selected_copies.clear();edits.active_copy=None;
                                            }
                                            apply(selected,active,Some(cell),
                                                modifiers.shift,modifiers.ctrl);
                                            *reveal=None;
                                            changed=true;
                                            ui.ctx().request_repaint();
                                        }

                                                                                response.context_menu(|ui|{
                                            if ui.button("Show in World Repository").clicked(){
                                                super::repo_jump::request(
                                                    ui.ctx(),
                                                    super::repo_jump::Target::Ground(cell)
                                                );
                                                ui.close();
                                            }
                                        });

                                        // Only scroll when a new viewport selection requests it.
                                        if *reveal==Some(cell) {
                                            response.scroll_to_me(
                                                Some(egui::Align::Center)
                                            );
                                            *reveal=None;
                                        }
                                    }
                                });
                        }
                    });
            }
        });

    changed
}


/// Frame an original 128x128 MAP ground cell, without editing it.
pub(super) fn frame_cell(
    view:&mut Viewport,
    model:&Model,
    hf:&WalkHeightfield,
    cell:[u8;2],
)->bool {
    for (quad,points) in hf.positions.chunks_exact(4).enumerate() {
        if cell_at(hf,quad)!=Some(cell) {continue;}

        let mut low=[f32::INFINITY;3];
        let mut high=[f32::NEG_INFINITY;3];

        for p in points {
            let pos=[p[0],-p[1],p[2]];
            for axis in 0..3 {
                low[axis]=low[axis].min(pos[axis]);
                high[axis]=high[axis].max(pos[axis]);
            }
        }

        let center=[0,1,2].map(|a|(low[a]+high[a])*0.5);
        let diagonal=[0,1,2]
            .map(|a|high[a]-low[a])
            .iter().map(|v|v*v).sum::<f32>().sqrt();
        let radius=(diagonal*0.5).max(64.0);

        view.frame_instance(
            model,center,radius,
            [0.0,0.0,0.0,1.0],
            [0.0,0.0,0.0],
        );
        view.version=view.version.wrapping_add(1);
        return true;
    }
    false
}
