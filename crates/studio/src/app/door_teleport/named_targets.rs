//! R639: project-local named cross-cell warp destination choices.
use super::*;
use std::hash::{Hash,Hasher};

#[derive(Clone,Debug,PartialEq,Eq)]
pub(super) struct Target {
    pub identity:String,
    pub scene:String,
    pub uid:u64,
    pub label:String,
    pub tile:[u8;2],
}

impl Target {
    fn caption(&self)->String {
        let variant=if self.identity.starts_with("copy:") {
            format!(" / {}",self.identity)
        } else {String::new()};
        format!("{} - {} ({}) / #{}{}",self.label,
            crate::place_name(&self.scene).unwrap_or(&self.scene),
            self.scene,self.uid,variant)
    }
    fn matches(&self,dest:&Destination)->bool {
        self.scene==dest.name && self.tile==[dest.entry_x&127,dest.entry_z&127]
    }
}

fn collect(identity:&str,scene:&str,records:&[Door])->Vec<Target> {
    records.iter().filter(|door|
        door.enabled && matches!(door.origin,Origin::New) &&
        matches!(door.kind,Kind::Script{destination:Some(_),..}) &&
        door.source.iter().all(|v|*v<128)
    ).map(|door|Target{
        identity:identity.into(),scene:scene.into(),uid:door.uid,
        label:door.visible_name(),tile:door.source,
    }).collect()
}

#[derive(Clone,Default)]
pub(super) struct Catalog {
    signature:Option<u64>,
    identity:String,
    saved:Vec<Target>,
    pub(super) errors:Vec<String>,
}

impl Catalog {
    pub fn refresh(&mut self,snapshots:Option<&BTreeMap<String,String>>,identity:&str) {
        let mut hasher=std::collections::hash_map::DefaultHasher::new();
        identity.hash(&mut hasher);
        if let Some(snapshots)=snapshots {
            for (key,text) in snapshots {
                key.hash(&mut hasher);
                for line in text.lines().filter(|line|line.starts_with("DOOR_EDIT\t")) {
                    line.hash(&mut hasher);
                }
            }
        }
        let signature=hasher.finish();
        if self.signature==Some(signature) {return;}
        self.signature=Some(signature);
        self.identity=identity.into();
        self.saved.clear();
        self.errors.clear();
        if let Some(snapshots)=snapshots {
            for (key,text) in snapshots {
                if key==identity {continue;}
                let rows=text.lines().filter_map(|line|line.strip_prefix("DOOR_EDIT\t")).collect::<Vec<_>>();
                if rows.is_empty() {continue;}
                if rows.len()!=1 {
                    self.errors.push(format!("{key}: multiple warp snapshots"));
                    continue;
                }
                match serde_json::from_str::<Saved>(rows[0]) {
                    Ok(saved) if saved.version==1 && !saved.scene.is_empty()=>{
                        self.saved.extend(collect(key,&saved.scene,&saved.records));
                    },
                    _=>self.errors.push(format!("{key}: saved warp data could not be read")),
                }
            }
        }
    }

    pub fn choices(&self,scene:&str,records:&[Door],source_uid:u64)->Vec<Target> {
        let mut choices=self.saved.clone();
        choices.extend(collect(&self.identity,scene,records).into_iter().filter(|t|t.uid!=source_uid));
        choices.sort_by_key(|t|(t.label.to_lowercase(),t.scene.clone(),t.identity.clone(),t.uid));
        choices
    }
}

fn receiving_arrival<'a>(target:&Target,retail:&'a [R622Scene])->Option<&'a R622Arrival> {
    retail.iter().find(|cell|cell.name==target.scene)?.arrivals.first()
}

fn select(dest:&mut Destination,target:&Target,retail:&[R622Scene])->bool {
    let Some(arrival)=receiving_arrival(target,retail) else {return false;};
    *dest=Destination{
        name:target.scene.clone(),index:arrival.index,
        entry_x:(arrival.x&128)|target.tile[0],
        entry_z:(arrival.z&128)|target.tile[1],facing:arrival.facing,
    };
    true
}

pub(super) fn picker(ui:&mut egui::Ui,uid:u64,dest:&mut Destination,
    retail:Option<&[R622Scene]>,choices:&[Target],search:&mut String,errors:&[String]) {
    ui.strong("Named Cross-Cell Warp");
    ui.horizontal(|ui|{
        ui.label("Find warp");
        ui.text_edit_singleline(search);
    });
    let matching=choices.iter().filter(|t|t.matches(dest)).collect::<Vec<_>>();
    let caption=if matching.len()==1 {matching[0].caption()}
        else if matching.len()>1 {"Multiple named warps at this arrival".into()}
        else {"Choose a named warp...".into()};
    egui::ComboBox::from_id_salt(("r639_named_warp",uid))
        .width(395.0).selected_text(caption).show_ui(ui,|ui|{
            let term=search.trim().to_lowercase();
            let mut count=0;
            for target in choices {
                let caption=target.caption();
                if !term.split_whitespace().all(|word|caption.to_lowercase().contains(word)) {continue;}
                count+=1;
                let available=retail.is_some_and(|r|receiving_arrival(target,r).is_some());
                let response=ui.add_enabled(available,
                    egui::Button::new(caption).selected(target.matches(dest)));
                if response.clicked() {
                    if let Some(retail)=retail {select(dest,target,retail);}
                    ui.close();
                }
                if !available {response.on_hover_text("This cell has no decoded arrival available yet.");}
            }
            if count==0 {ui.label("No matching custom cross-cell warps in this project.");}
        });
    ui.small("Uses the named trigger's position. Adjust arrival X/Z below if needed.");
    if !errors.is_empty() {
        ui.colored_label(egui::Color32::LIGHT_RED,"Some saved cell warp lists could not be read.")
            .on_hover_text(errors.join("\n"));
    }
    ui.separator();
}

impl State {
    pub(in crate::app) fn r639_refresh_named_warps(&mut self,snapshots:Option<&BTreeMap<String,String>>,identity:&str) {
        if self.marker_properties.is_some() {self.r639_named_catalog.refresh(snapshots,identity);}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn door(uid:u64,label:&str)->Door {
        Door{uid,label:label.into(),origin:Origin::New,source:[20,30],size:[1,1],enabled:true,
            kind:Kind::Script{slot:Some(7),op_pc:None,destination:Some(Destination{
                name:"map01".into(),index:99,entry_x:2,entry_z:3,facing:4,
            })}}
    }
    fn snapshot(scene:&str,records:Vec<Door>)->String {
        let state=State{scene:scene.into(),records,..Default::default()};
        state.embedded_record(scene)
    }
    #[test]
    fn target_uses_receiving_scene_index_instead_of_its_outgoing_destination() {
        let target=collect("original:town01","town01",&[door(1,"My Rim Elm Gate")]).remove(0);
        let retail=vec![R622Scene{name:"town01".into(),arrivals:vec![R622Arrival{
            from_scene:"map01".into(),index:5,x:129,z:2,facing:6,
        }]}];
        let mut dest=Destination{name:"old".into(),index:99,entry_x:0,entry_z:0,facing:0};
        assert!(select(&mut dest,&target,&retail));
        assert_eq!((dest.name.as_str(),dest.index,dest.entry_x,dest.entry_z,dest.facing),("town01",5,148,30,6));
        assert!(!select(&mut dest,&target,&[]));
        assert_eq!(dest.index,5);
    }
    #[test]
    fn saved_names_survive_cell_switches_and_active_records_replace_stale_snapshots() {
        let mut snapshots=BTreeMap::from([
            ("original:town01".into(),snapshot("town01",vec![door(1,"Old Name")])),
            ("original:conc".into(),snapshot("conc",vec![door(2,"Conkram Station")])),
        ]);
        let mut catalog=Catalog::default();
        catalog.refresh(Some(&snapshots),"original:town01");
        let live=vec![door(1,"Renamed Gate"),door(3,"Source")];
        let choices=catalog.choices("town01",&live,3);
        assert_eq!(choices.len(),2);
        assert!(choices.iter().any(|t|t.label=="Renamed Gate"));
        assert!(!choices.iter().any(|t|t.label=="Old Name"));
        snapshots.insert("original:conc".into(),snapshot("conc",vec![door(2,"Renamed Station")]));
        catalog.refresh(Some(&snapshots),"original:town01");
        assert!(catalog.choices("town01",&live,3).iter().any(|t|t.label=="Renamed Station"));
        snapshots.clear();
        catalog.refresh(Some(&snapshots),"original:town01");
        assert_eq!(catalog.choices("town01",&live,3).len(),1);
    }
    #[test]
    fn disabled_deleted_and_non_cross_cell_markers_are_excluded_and_duplicate_names_keep_identity() {
        let mut disabled=door(2,"Disabled");disabled.enabled=false;
        let mut local=door(3,"Local");local.kind=Kind::Local{half_x:2,half_z:2};
        assert_eq!(collect("original:town01","town01",&[door(1,"Gate"),disabled,local]).len(),1);
        let snapshots=BTreeMap::from([
            ("original:town01".into(),snapshot("town01",vec![door(1,"Gate")])),
            ("original:conc".into(),snapshot("conc",vec![door(1,"Gate")])),
        ]);
        let mut catalog=Catalog::default();catalog.refresh(Some(&snapshots),"original:map01");
        let choices=catalog.choices("map01",&[],0);
        assert_eq!(choices.len(),2);
        assert_ne!(choices[0].caption(),choices[1].caption());
    }
    #[test]
    fn bad_snapshots_are_reported_without_importing_partial_targets() {
        let snapshots=BTreeMap::from([("original:town01".into(),"DOOR_EDIT\t{bad}\n".into())]);
        let mut catalog=Catalog::default();catalog.refresh(Some(&snapshots),"original:conc");
        assert!(catalog.saved.is_empty());assert_eq!(catalog.errors.len(),1);
        catalog.refresh(None,"original:conc");assert!(catalog.errors.is_empty());
    }
}
