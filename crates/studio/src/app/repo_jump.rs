//! R621.1 - Universal source identity navigation.
//! Repository matches are exact; native records remain separately identified.

use super::*;
use eframe::egui;

#[derive(Clone,Debug)]
pub(super) enum Target {
    Draw(bool,usize),
    Mesh(usize),
    Ground([u8;2]),
    GroundCopy { source:[u8;2],uid:u64 },
    Collision(i32,i32),
    Record { kind:String,title:String,details:String },
}

pub(super) struct SourcePreview {
    pub source:String,
    pub kind:String,
    pub title:String,
    pub details:String,
    pub scene:Option<legaia_engine_scene::scene_assembly::AssembledScene>,
    pub model:Option<crate::raster::Model>,
}

fn id()->egui::Id {
    egui::Id::new("lck_r621_repository_jump_request")
}

pub(super) fn request(ctx:&egui::Context,target:Target) {
    ctx.data_mut(|data|data.insert_temp(id(),target));
    ctx.request_repaint();
}

fn take(ctx:&egui::Context)->Option<Target> {
    ctx.data_mut(|data|{
        let value=data.get_temp::<Target>(id());
        data.remove::<Target>(id());
        value
    })
}

pub(super) fn ground_quad(
    scene:&legaia_engine_scene::scene_assembly::AssembledScene,
    cell:[u8;2]
)->Option<(crate::raster::Model,String)> {
    let hf=scene.ground.as_ref()?;
    for quad in 0..hf.quad_count() {
        let base=quad*4;
        let p=hf.positions.get(base..base+4)?;
        let cx=(p[0][0]/128.0).round() as i32;
        let cz=(p[0][2]/128.0).round() as i32;
        if [cx,cz]!=[i32::from(cell[0]),i32::from(cell[1])] {
            continue;
        }

        let positions=p.iter()
            .map(|v|[v[0],-v[1],v[2]])
            .collect::<Vec<_>>();
        let mut center=[0.0_f32;3];
        for p in &positions {
            for axis in 0..3 {
                center[axis]+=p[axis]*0.25;
            }
        }

        let page=*hf.cba_tsb.get(base)?;
        let atlas=*hf.tile_ids.get(base)?;
        let model=crate::raster::Model {
            rest:positions.clone(),
            positions,
            object_ids:vec![0;4],
            uvs:hf.uvs.get(base..base+4)?.to_vec(),
            cba_tsb:hf.cba_tsb.get(base..base+4)?.to_vec(),
            modulation:hf.colors.get(base..base+4)?.to_vec(),
            fill:vec![[180;3];4],
            textured:vec![1;4],
            indices:vec![0,1,2,1,3,2],
            center,
            radius:128.0,
        };

        return Some((
            model,
            format!(
                "Native MAP 128x128 floor | tile ({},{}) | atlas {} | TPAGE {:04X} | CLUT {:04X}",
                cell[0],cell[1],atlas,page[1],page[0]
            )
        ));
    }
    None
}

impl App {
    pub(super) fn repo_jump_process(
        &mut self,
        ctx:&egui::Context,
        scene:Option<&legaia_engine_scene::scene_assembly::AssembledScene>,
        world:Option<&world_view::WorldModel>
    ) {
        let target=take(ctx).or_else(||self.repo_jump_pending.take());
        let Some(target)=target else {return;};
        let Some(scene)=scene else {
            self.studio.status=Some(
                "Select a source cell before locating its original record".into()
            );
            return;
        };

        let mut direct:Option<(
            String,
            global_repository::Kind,
            Option<(usize,usize)>
        )>=None;

        let mut native:Option<(
            String,String,String,Option<[u8;2]>
        )>=None;

        match &target {
            Target::Draw(terrain,index)=>{
                let mut src=scene.name.clone();
                let mut kind=if *terrain {
                    global_repository::Kind::Terrain(*index)
                } else if *index>=world_view::NPC_KEY_BASE {
                    global_repository::Kind::Npc(
                        *index-world_view::NPC_KEY_BASE
                    )
                } else {
                    global_repository::Kind::Placement(*index)
                };
                let mut face=None;

                if let Some(draw)=world.and_then(|w|w.at(*terrain,*index)) {
                    if let Some(uid)=draw.editor_uid {
                        if let Some(foreign)=world.and_then(|w|
                            w.foreign.iter().find(|f|f.uid==uid)
                        ) {
                            src=foreign.source.clone();
                            kind=match foreign.kind.as_str() {
                                "terrain"=>global_repository::Kind::Terrain(
                                    foreign.record
                                ),
                                "placement"=>global_repository::Kind::Placement(
                                    foreign.record
                                ),
                                "mesh"=>global_repository::Kind::Mesh(
                                    foreign.record
                                ),
                                encoded=>{
                                    face=floor_tiles::decode_floor_kind(encoded);
                                    global_repository::Kind::Mesh(foreign.record)
                                }
                            };
                        } else if let Some((t,i))=draw.source_ref {
                            kind=if t {
                                global_repository::Kind::Terrain(i)
                            } else {
                                global_repository::Kind::Placement(i)
                            };
                        }
                    }
                }
                direct=Some((src,kind,face));
            }

            Target::Mesh(slot)=>{
                direct=Some((
                    scene.name.clone(),
                    global_repository::Kind::Mesh(*slot),
                    None
                ));
            }

            Target::Ground(cell)=>{
                self.native_floor_jump_cell=Some((
                    scene.name.clone(),*cell
                ));
                self.editor_search.clear();
                native=Some((
                    "Native MAP ground".into(),
                    format!("Floor tile [{},{}]",cell[0],cell[1]),
                    "Original 128x128 scene ground; not a standalone TMD face.".into(),
                    Some(*cell)
                ));
            }

            Target::GroundCopy{source,uid}=>{
                native=Some((
                    "Copied MAP ground".into(),
                    format!("Ground copy #{uid} -> [{},{}]",source[0],source[1]),
                    "Original MAP tile retained by an independent editable copy.".into(),
                    Some(*source)
                ));
            }

            Target::Collision(x,z)=>{
                native=Some((
                    "MAP collision".into(),
                    format!("Collision box ({x},{z})"),
                    "Collision grid reference, not a reusable textured TMD.".into(),
                    None
                ));
            }

            Target::Record{kind,title,details}=>{
                native=Some((
                    kind.clone(),
                    title.clone(),
                    details.clone(),
                    None
                ));
            }
        }

        if let Some((source,kind,face))=direct {
            if let Some(id)=self.global_repository.iter().position(|asset|
                asset.source==source && asset.kind==kind
            ) {
                self.editor_search.clear();
                self.global_repository_selected=Some(id);
                self.repo_jump_focus_id=Some(id);
                self.global_repository_scroll_target=Some(id);
                self.global_repository_dialog=Some((id,true));
                self.global_repository_preview=None;
                self.global_floor_face=None;
                self.global_floor_scroll_target=None;
                self.repo_jump_native=None;

                if let Some((object,primitive))=face {
                    if let Some(actual)=self.global_repository[id].floor_faces
                        .iter().copied()
                        .find(|f|
                            f.object==object &&
                            f.primitive==primitive
                        )
                    {
                        let slot=match kind {
                            global_repository::Kind::Mesh(s)=>s,
                            _=>0
                        };
                        self.global_floor_face=Some((
                            source,slot,actual
                        ));
                        self.global_floor_scroll_target=Some((id,actual));
                    }
                }

                ctx.request_repaint();
                return;
            }

            if !self.global_repository_started ||
                self.global_repository_completed<self.global_repository_total
            {
                self.repo_jump_pending=Some(target);
                self.studio.status=Some(
                    "Waiting for World Repository source scan".into()
                );
                return;
            }

            native=Some((
                "Unindexed source".into(),
                format!("{:?} in {source}",kind),
                "No exact original repository asset matches this key. No visual approximation was substituted.".into(),
                None
            ));
        }

        if let Some((kind,title,mut details,ground))=native {
            let mut source_scene=None;
            let mut model=None;

            if let Some(cell)=ground {
                source_scene=self.studio.library.as_ref().and_then(|lib|
                    legaia_engine_world_views::scene_staging::assemble_field_scene(
                        &lib.index,&scene.name
                    ).ok()
                );

                if let Some(sc)=source_scene.as_ref() {
                    if let Some((tile,metadata))=ground_quad(sc,cell) {
                        model=Some(tile);
                        details.push_str("\n");
                        details.push_str(&metadata);
                    } else {
                        details.push_str(
                            "\nOriginal MAP ground tile not found in assembled source."
                        );
                    }
                } else {
                    details.push_str(
                        "\nSource scene could not be reassembled for textured preview."
                    );
                }
            }

            self.repo_jump_focus_id=None;
            self.repo_jump_native=Some(SourcePreview {
                source:scene.name.clone(),
                kind,
                title,
                details,
                scene:source_scene,
                model
            });
            self.repo_jump_reveal=true;
            self.repo_jump_view=Viewport::new(Shading::Textured);
            self.editor_search.clear();
            self.studio.status=Some(
                "World Repository native source opened without inferred TMD match".into()
            );
            ctx.request_repaint();
        }
    }

    pub(super) fn repo_jump_source_panel(&mut self,ui:&mut egui::Ui) {
        if let Some(view)=self.repo_jump_native.as_ref() {
            ui.group(|ui|{
                let label=ui.strong(format!(
                    "Native Source Records / {} / {} / {}",
                    view.source,view.kind,view.title
                ));
                if self.repo_jump_reveal {
                    label.scroll_to_me(Some(egui::Align::Center));
                    self.repo_jump_reveal=false;
                }
                ui.small(&view.details);
            });
        }
    }

    pub(super) fn repo_jump_source_window(&mut self,ctx:&egui::Context) {
        let Some(view)=self.repo_jump_native.as_ref() else {
            return;
        };

        let mut open=true;
        egui::Window::new(
            format!("World Repository Preview - {}",view.title)
        )
        .id(egui::Id::new("lck_r621_source_preview"))
        .default_size([660.0,610.0])
        .resizable(true)
        .open(&mut open)
        .show(ctx,|ui|{
            ui.heading(&view.title);
            ui.monospace(format!(
                "Native source: {} / {}",view.source,view.kind
            ));
            ui.label(&view.details);
            ui.separator();

            if let (Some(sc),Some(model))=(
                view.scene.as_ref(),view.model.as_ref()
            ) {
                ui.small(
                    "Original 128x128 MAP ground: UV, TPAGE, CLUT and source VRAM"
                );
                let available=ui.available_width().max(350.0);
                ui.allocate_ui(
                    egui::vec2(available,410.0),
                    |ui|{
                        self.repo_jump_view.show_limited(
                            ui,model,&sc.res.vram,800,600
                        );
                    }
                );
            } else {
                ui.small(
                    "Native record has no standalone 3D asset. Original metadata is shown without fabricating geometry."
                );
            }
        });

        if !open {
            self.repo_jump_native=None;
        }
    }
}

// R625 - Exact original cell metadata when no repository asset was indexed.
impl App {
    pub(super) fn r625_show_unindexed_cell(
        &mut self,
        source:String,
        ctx:&egui::Context
    ){
        self.global_repository_dialog=None;
        self.global_repository_selected=None;
        self.repo_jump_focus_id=None;
        self.global_repository_scroll_target=None;
        self.repo_jump_native=Some(SourcePreview {
            source:source.clone(),
            kind:"Original cell / variant".into(),
            title:format!("Original source {source}"),
            details:format!(
                "Original PS1 scene ID: {source}\n\
                 No independent World Repository object or native MAP floor \
                 was indexed for this source variant.\n\
                 No alternative scene or inferred object was substituted."
            ),
            scene:None,
            model:None,
        });
        self.repo_jump_reveal=true;
        self.repo_jump_view=Viewport::new(Shading::Textured);
        self.studio.status=Some(format!(
            "Original source metadata opened: {source}"
        ));
        ctx.request_repaint();
    }
}
