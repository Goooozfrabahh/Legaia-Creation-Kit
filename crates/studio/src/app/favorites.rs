//! R622.1 - Persistent favorites for original World Repository assets.
use std::{fs,path::PathBuf};
use serde::{Serialize,Deserialize};
use eframe::egui;
use super::{global_repository::{Asset,Kind},floor_tiles::Face};

#[derive(Clone,Debug,Serialize,Deserialize)]
pub(super) struct Favorite {
    source:String,
    key:String,
    class:String,
    index:usize,
    face:Option<[usize;2]>,
    palette:Option<[u16;2]>,
    shape:String,
    label:String,
    #[serde(default)]
    native:Option<super::native_floor_repository::Key>,
}

impl Favorite {
    pub(super) fn new(
        asset:&Asset,
        face:Option<Face>,
        names:&crate::asset_catalog::Catalog
    )->Self {
        let (class,index)=match asset.kind {
            Kind::Placement(i)=>("placement",i),
            Kind::Terrain(i)=>("terrain",i),
            Kind::Mesh(i)=>("mesh",i),
            Kind::Npc(i)=>("npc",i),
        };
        Self {
            source:asset.source.clone(),
            key:asset.key.clone(),
            class:class.into(),
            index,
            face:face.map(|f|[f.object,f.primitive]),
            palette:face.map(|f|[f.cba,f.tsb]),
            shape:face.map(|f|f.shape.into()).unwrap_or_default(),
            label:asset.display_name(names),
            native:None,
        }
    }

    pub(super) fn native(
        tile:&super::native_floor_repository::Tile
    )->Self{
        let key=&tile.key;
        Self{
            source:key.source.clone(),
            key:format!(
                "native:{}:{}:{}:{}:{:?}",
                key.source,key.atlas,key.tpage,key.clut,key.uvs
            ),
            class:"native_map_floor".into(),
            index:0,
            face:None,
            palette:Some([key.clut,key.tpage]),
            shape:"Flat Squares / 128 x 128".into(),
            label:format!("Native atlas {}",key.atlas),
            native:Some(key.clone()),
        }
    }

    fn equal(&self,b:&Self)->bool {
        self.source==b.source &&
        self.key==b.key &&
        self.class==b.class &&
        self.index==b.index &&
        self.face==b.face &&
        self.palette==b.palette &&
        self.native==b.native
    }

    fn group(&self)->usize {
        if self.native.is_some(){return 0;}
        if self.face.is_some() {
            if self.shape=="Flat Squares / 128 x 128" {0}else{1}
        }else{
            match self.class.as_str() {
                "placement"=>2,
                "terrain"=>3,
                "mesh"=>4,
                "npc"=>5,
                _=>4
            }
        }
    }

    fn resolve(&self,assets:&[Asset])->Option<(usize,Option<Face>)>{
        let (id,asset)=assets.iter().enumerate().find(|(_,a)|{
            if a.source!=self.source || a.key!=self.key {
                return false;
            }
            match (self.class.as_str(),a.kind) {
                ("placement",Kind::Placement(i)) |
                ("terrain",Kind::Terrain(i)) |
                ("mesh",Kind::Mesh(i)) |
                ("npc",Kind::Npc(i))=>i==self.index,
                _=>false
            }
        })?;

        let face=if let Some([object,primitive])=self.face {
            Some(*asset.floor_faces.iter().find(|f|
                f.object==object &&
                f.primitive==primitive &&
                self.palette.is_none_or(|p|p==[f.cba,f.tsb])
            )?)
        }else{None};

        Some((id,face))
    }
}

#[derive(Serialize,Deserialize)]
struct Data {
    version:u32,
    entries:Vec<Favorite>,
}

pub(super) struct Store {
    entries:Vec<Favorite>,
    path:PathBuf,
    error:Option<String>,
}

impl Store {
    pub(super) fn load()->Self {
        let path=dirs::config_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("Legaia Creation Kit")
            .join("world_repository_favorites.json");

        let mut result=Self {
            entries:Vec::new(),
            path,
            error:None,
        };

        if result.path.exists() {
            let loaded=fs::read_to_string(&result.path)
                .map_err(|e|e.to_string())
                .and_then(|text|
                    serde_json::from_str::<Data>(&text)
                        .map_err(|e|e.to_string())
                );
            match loaded {
                Ok(data) if data.version==1=>{
                    result.entries=data.entries;
                }
                Ok(data)=>result.error=Some(format!(
                    "Unsupported Favorites file version {}",
                    data.version
                )),
                Err(e)=>result.error=Some(e),
            }
        }
        result
    }

    pub(super) fn has(&self,f:&Favorite)->bool{
        self.entries.iter().any(|x|x.equal(f))
    }

    pub(super) fn toggle(&mut self,f:Favorite)->Result<bool,String>{
        if let Some(e)=&self.error {
            return Err(format!("Favorites file not loaded: {e}"));
        }

        let before=self.entries.clone();
        let added=if let Some(i)=self.entries.iter()
            .position(|x|x.equal(&f)){
            self.entries.remove(i);
            false
        }else{
            self.entries.push(f);
            true
        };

        if let Err(e)=self.save() {
            self.entries=before;
            return Err(e);
        }
        Ok(added)
    }

    fn save(&self)->Result<(),String>{
        let folder=self.path.parent().ok_or("Invalid Favorites path")?;
        fs::create_dir_all(folder).map_err(|e|e.to_string())?;

        let bytes=serde_json::to_vec_pretty(&Data{
            version:1,entries:self.entries.clone()
        }).map_err(|e|e.to_string())?;

        let tmp=self.path.with_extension("json.tmp");
        fs::write(&tmp,bytes).map_err(|e|e.to_string())?;

        if self.path.exists() {
            fs::copy(&self.path,self.path.with_extension("json.bak"))
                .map_err(|e|e.to_string())?;
        }

        if fs::rename(&tmp,&self.path).is_err() {
            fs::copy(&tmp,&self.path).map_err(|e|e.to_string())?;
            let _=fs::remove_file(&tmp);
        }
        Ok(())
    }
}

pub(super) enum Action {
    NativePreview(super::native_floor_repository::Key),
    NativeDrag(super::native_floor_repository::Key),
    Open(usize,Option<Face>,bool),
    Drag(usize,Option<Face>),
    Remove(Favorite),
}

pub(super) fn draw(
    ui:&mut egui::Ui,
    store:&Store,
    assets:&[Asset],
    names:&crate::asset_catalog::Catalog,
    native_tiles:&[super::native_floor_repository::Tile]
)->Option<Action> {
    let mut action=None;
    let groups=[
        "128 x 128 Floor Tiles",
        "Other Floor Pieces",
        "World Objects",
        "Terrain & Decoration",
        "Reusable Geometry / TMD",
        "Characters & NPCs"
    ];

    egui::CollapsingHeader::new(format!(
        "Favorites ({})",store.entries.len()
    ))
    .id_salt("r622_favorites_root")
    .default_open(true)
    .show(ui,|ui|{
        if let Some(error)=&store.error {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                format!("Favorites load failed: {error}")
            );
        }

        if store.entries.is_empty() {
            ui.small("Right-click a repository item to add it.");
        }

        for (group,title) in groups.iter().enumerate(){
            let items=store.entries.iter()
                .filter(|e|e.group()==group)
                .collect::<Vec<_>>();

            egui::CollapsingHeader::new(format!(
                "{title} ({})",items.len()
            ))
            .id_salt(("r622_favorites_group",group))
            .default_open(false)
            .show(ui,|ui|{
                for favorite in items {
                    if let Some(key)=favorite.native.as_ref(){
                        if let Some(tile)=native_tiles.iter().find(
                            |t|&t.key==key
                        ){
                            let text=format!(
                                "★ Atlas {} [{}] TPAGE {:04X} CLUT {:04X}",
                                key.atlas,key.source,key.tpage,key.clut
                            );
                            let response=ui.add(
                                egui::Button::new(text)
                                    .sense(egui::Sense::click_and_drag())
                            );
                            if response.clicked(){
                                action=Some(Action::NativePreview(key.clone()));
                            }
                            if response.drag_started(){
                                action=Some(Action::NativeDrag(key.clone()));
                            }
                            response.context_menu(|ui|{
                                if ui.button("Preview").clicked(){
                                    action=Some(Action::NativePreview(key.clone()));
                                    ui.close();
                                }
                                if ui.button("Remove from Favorites").clicked(){
                                    action=Some(Action::Remove((*favorite).clone()));
                                    ui.close();
                                }
                                ui.small(format!(
                                    "{} original MAP cells",tile.cells.len()
                                ));
                            });
                        }else{
                            ui.label(format!(
                                "Unavailable native floor [{}] atlas {}",
                                key.source,key.atlas
                            )).context_menu(|ui|{
                                if ui.button("Remove from Favorites").clicked(){
                                    action=Some(Action::Remove((*favorite).clone()));
                                    ui.close();
                                }
                            });
                        }
                        continue;
                    }

                    if let Some((id,face))=favorite.resolve(assets) {
                        let asset=&assets[id];
                        let mut label=format!(
                            "★ {} [{}]",
                            asset.display_name(names),asset.source
                        );

                        if let Some(f)=face {
                            label.push_str(&format!(
                                " / {} / TMD {} OBJ {} Face {}",
                                f.shape,favorite.index,
                                f.object,f.primitive
                            ));
                        }

                        let response=ui.add(
                            egui::Button::new(label)
                                .sense(egui::Sense::click_and_drag())
                        );

                        if response.clicked() {
                            action=Some(Action::Open(id,face,true));
                        }
                        if response.drag_started() {
                            action=Some(Action::Drag(id,face));
                        }

                        response.context_menu(|ui|{
                            if ui.button("Preview").clicked(){
                                action=Some(Action::Open(id,face,true));
                                ui.close();
                            }
                            if ui.button("Properties").clicked(){
                                action=Some(Action::Open(id,face,false));
                                ui.close();
                            }
                            if ui.button("Remove from Favorites").clicked(){
                                action=Some(Action::Remove((*favorite).clone()));
                                ui.close();
                            }
                        });
                    }else{
                        ui.label(format!(
                            "Unavailable: {} [{}]",
                            favorite.label,favorite.source
                        )).context_menu(|ui|{
                            if ui.button("Remove from Favorites").clicked(){
                                action=Some(Action::Remove((*favorite).clone()));
                                ui.close();
                            }
                        });
                    }
                }
            });
        }
    });

    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn different_faces_have_different_identity(){
        let a=Favorite {
            source:"conc".into(),
            key:"mesh".into(),
            class:"mesh".into(),
            index:2,
            face:Some([0,1]),
            palette:Some([8,12]),
            shape:"Flat Squares / 128 x 128".into(),
            label:"brick".into(),
            native:None,
        };
        let mut b=a.clone();
        b.face=Some([0,2]);
        assert!(!a.equal(&b));
        b=a.clone();
        b.label="renamed".into();
        assert!(a.equal(&b));
    }
}