//! LCK-only cell identities. Retail CDNAMEs, PROT resources, and the PS1 disc
//! are never renamed or duplicated: a project copy is an independent overlay
//! against its original retail scene, with a distinct persistent editor ID.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CellSelection {
    Original(String),
    Copy(u64),
}

#[derive(Debug, Clone)]
pub(super) struct CellCopy {
    pub id: u64,
    pub source: String,
    pub name: String,
}

#[derive(Debug, Default, Clone)]
pub(super) struct CellStore {
    pub dir: Option<PathBuf>,
    pub names: BTreeMap<String, String>,
    pub copies: BTreeMap<u64, CellCopy>,
    pub next_id: u64,
    snapshot:Option<String>,
}

fn hex(s: &str) -> String {
    s.as_bytes().iter().map(|b| format!("{b:02X}")).collect()
}
fn unhex(s: &str) -> Result<String, String> {
    if s.len() > 1024 || s.len() % 2 != 0 { return Err("Invalid cell label encoding".into()); }
    let raw = s.as_bytes().chunks_exact(2).map(|pair| {
        std::str::from_utf8(pair).ok().and_then(|x| u8::from_str_radix(x, 16).ok())
            .ok_or_else(|| "Invalid cell label hex".to_owned())
    }).collect::<Result<Vec<_>, _>>()?;
    String::from_utf8(raw).map_err(|_| "Invalid UTF-8 cell label".to_owned())
}
fn valid_label(s: &str) -> Result<String, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 80 || trimmed.len() > 256 ||
        trimmed.chars().any(char::is_control) {
        return Err("Cell name must have 1-80 printable characters (max 256 bytes)".into());
    }
    Ok(trimmed.to_owned())
}
pub(super) fn duplicate_label(base:&str)->String {
    let mut base=base.to_owned();
    loop {let label=format!("{base} Copy");if valid_label(&label).is_ok(){return label;}if base.pop().is_none(){return "Cell Copy".into();}}
}
fn hash_source(path: &Path) -> String {
    // Stable FNV-1a rather than Rust's version-dependent DefaultHasher.
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in path.to_string_lossy().to_lowercase().as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016X}")
}
fn project_dir(source: &Path) -> Result<PathBuf, String> {
    let root = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .map(PathBuf::from)
        .ok_or("Cannot locate LOCALAPPDATA or APPDATA for editor-only cell copies")?;
    Ok(root.join("LegaiaStudio").join("CellProjects").join(hash_source(source)))
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let old=match std::fs::read(path) {Ok(bytes)=>Some(bytes),Err(e) if e.kind()==std::io::ErrorKind::NotFound=>None,Err(e)=>return Err(e.to_string())};
    crate::asset_catalog::atomic_write(path,bytes,old.as_deref())
}
impl CellStore {
    pub fn open(source: Option<&Path>) -> Result<Self, String> {
        let Some(source) = source else { return Ok(Self::default()); };
        let dir = project_dir(source)?;
        Self::open_dir(dir)
    }
    fn open_dir(dir:PathBuf)->Result<Self,String> {
        let file = dir.join("cells.lckcatalog");
        let mut store = Self { dir: Some(dir), next_id: 1, ..Default::default() };
        if !file.exists() { return Ok(store); }
        let input = std::fs::read_to_string(&file).map_err(|e| format!("Read {}: {e}",file.display()))?;
        store.snapshot=Some(input.clone());
        if input.len() > 1_000_000 { return Err("Cell catalog is too large".into()); }
        if input.lines().next() != Some("LCK_CELLS_V1") {
            return Err("Unsupported LCK cell catalog version (not overwritten)".into());
        }
        for (line_no, line) in input.lines().skip(1).enumerate() {
            let cols = line.split('\t').collect::<Vec<_>>();
            match cols.as_slice() {
                ["ORIGINAL", source, name] => {
                    let key=unhex(source)?;
                    let value=valid_label(&unhex(name)?)?;
                    if store.names.insert(key,value).is_some() { return Err("Duplicate original cell key".into()); }
                }
                ["COPY", id, source, name] => {
                    let id=id.parse::<u64>().map_err(|_| "Invalid cell copy ID")?;
                    if id==0 {return Err("Invalid zero copy ID".into());}
                    let source=unhex(source)?;
                    let name=valid_label(&unhex(name)?)?;
                    if store.copies.insert(id,CellCopy{id,source,name}).is_some() {
                        return Err("Duplicate copy ID in cell catalog".into());
                    }
                    store.next_id=store.next_id.max(id.checked_add(1).ok_or("Cell ID exhausted")?);
                }
                _ => return Err(format!("Invalid cell catalog record on line {}",line_no+2)),
            }
        }
        Ok(store)
    }
    pub fn source<'a>(&'a self, id: &'a CellSelection) -> Option<&'a str> {
        match id {
            CellSelection::Original(source)=>Some(source),
            CellSelection::Copy(uid)=>self.copies.get(uid).map(|x|x.source.as_str()),
        }
    }
    pub fn label(&self, id: &CellSelection) -> Option<String> {
        match id {
            CellSelection::Original(source) => Some(self.names.get(source).cloned()
                .unwrap_or_else(|| crate::place_name(source).unwrap_or(source).to_owned())),
            CellSelection::Copy(uid) => self.copies.get(uid).map(|c|c.name.clone()),
        }
    }
    fn manifest(&self) -> String {
        let mut text=String::from("LCK_CELLS_V1\n");
        for (source,name) in &self.names {text.push_str(&format!("ORIGINAL\t{}\t{}\n",hex(source),hex(name)));}
        for copy in self.copies.values() {
            text.push_str(&format!("COPY\t{}\t{}\t{}\n",copy.id,hex(&copy.source),hex(&copy.name)));
        }
        text
    }
    fn save(&mut self) -> Result<(),String> {
        let Some(dir)=self.dir.as_ref() else {return Err("Open a disc image before creating project cells".into());};
        let manifest=self.manifest();
        crate::asset_catalog::atomic_write(&dir.join("cells.lckcatalog"),manifest.as_bytes(),self.snapshot.as_deref().map(str::as_bytes))?;
        self.snapshot=Some(manifest);Ok(())
    }
    pub fn rename(&mut self, selected:&CellSelection, value:&str) -> Result<(),String> {
        let value=crate::asset_catalog::display_name(value)?;
        let mut draft=self.clone();
        match selected {
            CellSelection::Original(source)=>{if let Some(value)=value {draft.names.insert(source.clone(),value);}else{draft.names.remove(source);}}
            CellSelection::Copy(id)=>{
                let entry=draft.copies.get_mut(id).ok_or("Missing cell copy")?;
                entry.name=value.unwrap_or_else(||format!("{} Copy #{}",crate::place_name(&entry.source).unwrap_or(&entry.source),entry.id));
            }
        }
        draft.save()?;
        *self=draft;
        Ok(())
    }
    fn overlay_path(&self,id:u64)->Result<PathBuf,String> {
        Ok(self.dir.as_ref().ok_or("Cell catalog has no storage")?
            .join(format!("copy_{id:016X}.lckscene")))
    }
    pub fn read_overlay(&self,id:u64)->Result<String,String> {
        let source=&self.copies.get(&id).ok_or("Unknown project cell")?.source;
        let path=self.overlay_path(id)?;
        let value=std::fs::read_to_string(&path)
            .map_err(|e|format!("Read {}: {e}",path.display()))?;
        if !value.starts_with(&format!("LCK_SCENE_V1\t{source}\n")) {
            return Err("Cell copy overlay does not match its retail source".into());
        }
        Ok(value)
    }
    pub fn write_overlay(&self,id:u64,overlay:&str)->Result<(),String> {
        let source=&self.copies.get(&id).ok_or("Unknown project cell")?.source;
        if overlay.len()>2_000_000 || !overlay.starts_with(&format!("LCK_SCENE_V1\t{source}\n")) {
            return Err("Overlay is oversized or does not match the source cell".into());
        }
        let path=self.overlay_path(id)?;
        // Avoid repeated disk writes when nothing has changed.
        if std::fs::read_to_string(&path).ok().as_deref()==Some(overlay) {return Ok(());}
        atomic_write(&path,overlay.as_bytes())
    }
    pub fn duplicate(&mut self,source:String,name:String,overlay:&str)->Result<u64,String> {
        let name=valid_label(&name)?;
        let id=self.next_id.max(1);
        let next=id.checked_add(1).ok_or("Maximum number of cell copies reached")?;
        if self.copies.contains_key(&id) {return Err("Cell ID collision".into());}
        let mut draft=self.clone();draft.next_id=next;
        draft.copies.insert(id,CellCopy{id,source,name});
        if overlay.len()>2_000_000 || !overlay.starts_with(&format!("LCK_SCENE_V1\t{}\n",draft.copies[&id].source)) {
            return Err("Overlay is oversized or does not match the source cell".into());
        }
        // A stale session must never replace another session's copy at the
        // same ID before the manifest conflict is detected.
        crate::asset_catalog::atomic_write(&draft.overlay_path(id)?,overlay.as_bytes(),None)?;
        if let Err(error)=draft.save() {
            let _=std::fs::remove_file(draft.overlay_path(id)?);
            return Err(error);
        }
        *self=draft;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cell_names_restart_defaults_and_stale_sessions_preserve_source() {
        let dir=tempfile::tempdir().unwrap();let mut store=CellStore::open_dir(dir.path().to_owned()).unwrap();
        let original=CellSelection::Original("town01".into());
        store.rename(&original,"Rim Elm - Workshop").unwrap();
        let id=store.duplicate("town01".into(),"Independent cell".into(),"LCK_SCENE_V1\ttown01\n").unwrap();
        store.rename(&CellSelection::Copy(id),"Rim Elm - Interior 🏡").unwrap();
        let mut restarted=CellStore::open_dir(dir.path().to_owned()).unwrap();
        assert_eq!(restarted.label(&original).as_deref(),Some("Rim Elm - Workshop"));
        assert_eq!(restarted.source(&CellSelection::Copy(id)),Some("town01"));
        assert_eq!(restarted.label(&CellSelection::Copy(id)).as_deref(),Some("Rim Elm - Interior 🏡"));
        let mut stale=CellStore::open_dir(dir.path().to_owned()).unwrap();
        restarted.rename(&original,"Latest name").unwrap();
        assert!(stale.rename(&original,"Stale name").is_err());
        assert_eq!(stale.label(&original).as_deref(),Some("Rim Elm - Workshop"));
        restarted.rename(&original,"").unwrap();restarted.rename(&CellSelection::Copy(id),"").unwrap();
        let fresh=CellStore::open_dir(dir.path().to_owned()).unwrap();
        assert_eq!(fresh.label(&original),CellStore::default().label(&original));
        assert!(fresh.label(&CellSelection::Copy(id)).unwrap().contains(&format!("#{id}")));
        assert_eq!(fresh.read_overlay(id).unwrap(),"LCK_SCENE_V1\ttown01\n");
    }
    #[test]
    fn stale_copy_allocation_never_overwrites_an_existing_overlay() {
        let dir=tempfile::tempdir().unwrap();let mut first=CellStore::open_dir(dir.path().to_owned()).unwrap();
        let mut stale=CellStore::open_dir(dir.path().to_owned()).unwrap();
        let overlay="LCK_SCENE_V1\ttown01\n# preserved\n";
        let id=first.duplicate("town01".into(),"First".into(),overlay).unwrap();
        assert!(stale.duplicate("town01".into(),"Second".into(),"LCK_SCENE_V1\ttown01\n").is_err());
        assert_eq!(first.read_overlay(id).unwrap(),overlay);
    }
    #[test]
    fn names_preserve_unicode() {
        let n="Rim Elm — House";
        assert_eq!(unhex(&hex(n)).unwrap(),n);
        assert!(valid_label("  ").is_err());
        assert!(valid_label("New\nCell").is_err());
        assert!(valid_label(&duplicate_label(&"A".repeat(80))).is_ok());
        assert!(valid_label(&duplicate_label(&"🏡".repeat(64))).is_ok());
    }
    #[test]
    fn stable_disc_fingerprint() {
        assert_eq!(hash_source(Path::new("D:/TEST/LO.img")),hash_source(Path::new("d:/test/lo.img")));
    }
    #[test]
    fn roundtrip_manifest() {
        let mut store=CellStore::default();
        store.names.insert("town01".into(),"Rim Elm alternate".into());
        store.copies.insert(3,CellCopy{id:3,source:"town01".into(),name:"Rim Elm - Copy".into()});
        let value=store.manifest();
        assert!(value.starts_with("LCK_CELLS_V1\n"));
        assert!(value.contains(&hex("Rim Elm - Copy")));
    }
}
