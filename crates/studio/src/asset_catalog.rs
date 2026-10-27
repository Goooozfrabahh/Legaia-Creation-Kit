//! Evidence-bearing editor metadata. Never writes native game resources.
//! V1 names are imported without claiming that a user label is verified.
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

const HEADER: &str = "LEGAIA_ASSET_CATALOG_V2";

pub(crate) fn display_name(value:&str)->Result<Option<String>,String> {
    let value=value.trim();if value.is_empty(){return Ok(None);}
    if value.chars().count()>80 || value.len()>256 || value.chars().any(char::is_control) {
        return Err("Name must contain 1-80 printable characters (at most 256 UTF-8 bytes)".into());
    }
    Ok(Some(value.into()))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Confidence {
    #[default]
    UserAssigned,
    Unidentified,
    Probable,
    Verified,
}
impl Confidence {
    pub const ALL: [Self; 4] = [
        Self::UserAssigned,
        Self::Unidentified,
        Self::Probable,
        Self::Verified,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::UserAssigned => "User assigned",
            Self::Unidentified => "Unidentified",
            Self::Probable => "Probable",
            Self::Verified => "Verified",
        }
    }
    fn parse(s: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|c| c.label() == s)
            .ok_or_else(|| format!("Unknown identification status: {s}"))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Record {
    pub name: String,
    pub confidence: Confidence,
    pub category: String,
    pub aliases: String,
    pub evidence: String,
}
impl Record {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("A display name is required".into());
        }
        if matches!(self.confidence, Confidence::Probable | Confidence::Verified)
            && self.evidence.trim().is_empty()
        {
            return Err("Probable and verified identifications require evidence".into());
        }
        Ok(())
    }
}

/// Snapshot of a project-side database, with optimistic conflict detection.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub records: BTreeMap<String, Record>,
    snapshot: Option<String>,
}
impl Catalog {
    pub fn get(&self, key: &str) -> Option<&String> {
        self.records.get(key).map(|r| &r.name)
    }
    pub fn record(&self, key: &str) -> Record {
        self.records.get(key).cloned().unwrap_or_default()
    }
    pub fn search_text(&self, key: &str) -> String {
        match self.records.get(key) {
            Some(r) => format!(
                "{key} {} {} {} {}",
                r.name,
                r.category,
                r.aliases,
                r.confidence.label()
            ),
            None => key.to_owned(),
        }
    }
    pub fn load(path: &Path, legacy: &Path) -> Result<Self, String> {
        if let Some(text) = read_optional(path)? {
            let mut catalog = Self::parse(&text)?;
            catalog.snapshot = Some(text);
            return Ok(catalog);
        }
        let Some(text) = read_optional(legacy)? else {
            return Ok(Self::default());
        };
        let mut lines = text.lines();
        if lines.next().map(|s| s.trim_start_matches('\u{feff}')) != Some("LEGAIA_OBJECT_NAMES_V1")
        {
            return Err(format!(
                "Invalid legacy name header in {}",
                legacy.display()
            ));
        }
        let mut catalog = Self::default();
        for (i, line) in lines.enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let (key, name) = line
                .split_once('\t')
                .ok_or_else(|| format!("Invalid legacy row {}", i + 2))?;
            if key.is_empty() || name.trim().is_empty() || name.contains('\t') {
                return Err(format!("Invalid legacy row {}", i + 2));
            }
            let record = Record {
                name: name.to_owned(),
                evidence: "Imported from V1 user names; identification not independently verified"
                    .into(),
                ..Record::default()
            };
            if catalog.records.insert(key.to_owned(), record).is_some() {
                return Err(format!("Duplicate legacy key at row {}", i + 2));
            }
        }
        Ok(catalog)
    }
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        if lines.next().map(|s| s.trim_start_matches('\u{feff}')) != Some(HEADER) {
            return Err("Unknown asset catalog version; refusing to overwrite it".into());
        }
        let mut result = Self::default();
        for (i, line) in lines.enumerate() {
            if line.is_empty() {
                continue;
            }
            let fields = line
                .split('\t')
                .map(unescape)
                .collect::<Result<Vec<_>, _>>()?;
            if fields.len() != 6 || fields[0].is_empty() {
                return Err(format!("Invalid catalog row {}", i + 2));
            }
            let record = Record {
                name: fields[1].clone(),
                confidence: Confidence::parse(&fields[2])?,
                category: fields[3].clone(),
                aliases: fields[4].clone(),
                evidence: fields[5].clone(),
            };
            record
                .validate()
                .map_err(|e| format!("Row {}: {e}", i + 2))?;
            if fields[0].starts_with("visual-v1:") || fields[0].starts_with("npc-asset-v1:") {display_name(&record.name)?.ok_or("A display name is required")?;}
            if result.records.insert(fields[0].clone(), record).is_some() {
                return Err(format!("Duplicate key at row {}", i + 2));
            }
        }
        Ok(result)
    }
    pub fn serialize(&self) -> Result<String, String> {
        let mut text = format!("{HEADER}\n");
        for (key, r) in &self.records {
            if key.is_empty() {
                return Err("An asset key is required".into());
            }
            r.validate()?;
            if key.starts_with("visual-v1:") || key.starts_with("npc-asset-v1:") {display_name(&r.name)?.ok_or("A display name is required")?;}
            text.push_str(
                &[
                    key.as_str(),
                    &r.name,
                    r.confidence.label(),
                    &r.category,
                    &r.aliases,
                    &r.evidence,
                ]
                .into_iter()
                .map(escape)
                .collect::<Vec<_>>()
                .join("\t"),
            );
            text.push('\n');
        }
        Ok(text)
    }
    /// Serialize first, retain a last-good backup, then replace in one rename.
    /// A sibling lock serializes cooperating LCK writers. Stale snapshots fail.
    pub fn save(&mut self, path: &Path) -> Result<(), String> {
        let text = self.serialize()?;
        atomic_write(path,text.as_bytes(),self.snapshot.as_deref().map(str::as_bytes))?;
        self.snapshot = Some(text);
        Ok(())
    }
}

/// Shared metadata writer: sibling lock, expected bytes, durable temporary
/// file and one replace operation. Never remove the destination first.
pub(crate) fn atomic_write(path:&Path,bytes:&[u8],expected:Option<&[u8]>)->Result<(),String> {
        if let Some(parent)=path.parent() {fs::create_dir_all(parent).map_err(|e|e.to_string())?;}
        let lock_path = sidecar(path, ".lock");
        let lock = OpenOptions::new().write(true).create_new(true).open(&lock_path)
            .map_err(|e| format!("Cannot lock catalog {}: {e}. If no editor is running, remove the stale .lock file.", path.display()))?;
        let _guard = Lock {
            file: Some(lock),
            path: lock_path,
        };
        let current=match fs::read(path) {Ok(bytes)=>Some(bytes),Err(e) if e.kind()==ErrorKind::NotFound=>None,Err(e)=>return Err(e.to_string())};
        if current.as_deref() != expected {
            return Err("Catalog changed on disk. Restart LCK before editing names to avoid overwriting another session.".into());
        }
        let temp = sidecar(path, ".pending");
        let result = (|| -> Result<(), String> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .map_err(|e| format!("Cannot create {}: {e}", temp.display()))?;
            let write_result = (|| -> std::io::Result<()> {
                file.write_all(bytes)?;
                file.sync_all()
            })();
            drop(file);
            if let Err(e) = write_result {
                let _ = fs::remove_file(&temp);
                return Err(e.to_string());
            }
            let replace_result = (|| -> std::io::Result<()> {
                if current.is_some() {
                    fs::copy(path, sidecar(path, ".bak"))?;
                }
                fs::rename(&temp, path)
            })();
            if let Err(e) = replace_result {
                let _ = fs::remove_file(&temp);
                return Err(e.to_string());
            }
            Ok(())
        })();
        result?;
        Ok(())
}
fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Cannot read {}: {e}", path.display())),
    }
}
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut p = path.as_os_str().to_os_string();
    p.push(suffix);
    PathBuf::from(p)
}
struct Lock {
    file: Option<fs::File>,
    path: PathBuf,
}
impl Drop for Lock {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = fs::remove_file(&self.path);
    }
}
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}
fn unescape(s: &str) -> Result<String, String> {
    let mut chars = s.chars();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        out.push(match chars.next() {
            Some('\\') => '\\',
            Some('t') => '\t',
            Some('n') => '\n',
            Some('r') => '\r',
            _ => return Err("Invalid catalog escape sequence".into()),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "lck-catalog-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> PathBuf {
            self.0.join("catalog.tsv")
        }
        fn legacy(&self) -> PathBuf {
            self.0.join("legacy.tsv")
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn record(name: &str) -> Record {
        Record {
            name: name.into(),
            ..Record::default()
        }
    }
    #[test]
    fn roundtrip_escaping_and_unicode() {
        let mut c = Catalog::default();
        c.records.insert(
            "cell:a|placed:1".into(),
            Record {
                name: "Door α\\β\t\n".into(),
                confidence: Confidence::Verified,
                evidence: "Preview\nfront and back".into(),
                category: "Architecture".into(),
                aliases: "Entrance".into(),
            },
        );
        assert_eq!(
            Catalog::parse(&c.serialize().unwrap()).unwrap().records,
            c.records
        );
    }
    #[test]
    fn rejects_malformed_and_duplicate_data() {
        for text in [
            "wrong\na\tb",
            "LEGAIA_ASSET_CATALOG_V2\na\tb",
            "LEGAIA_ASSET_CATALOG_V2\na\tb\tVerified\t\t\t",
            "LEGAIA_ASSET_CATALOG_V2\na\tb\tMystery\t\t\t",
            "LEGAIA_ASSET_CATALOG_V2\na\tb\tUser assigned\t\t\t\na\tc\tUser assigned\t\t\t",
        ] {
            assert!(Catalog::parse(text).is_err());
        }
    }
    #[test]
    fn legacy_migration_preserves_original_and_confidence() {
        let d = Scratch::new();
        let old = "LEGAIA_OBJECT_NAMES_V1\nmesh:1\tDoor\n";
        fs::write(d.legacy(), old).unwrap();
        let mut c = Catalog::load(&d.path(), &d.legacy()).unwrap();
        assert_eq!(c.record("mesh:1").confidence, Confidence::UserAssigned);
        c.save(&d.path()).unwrap();
        assert_eq!(fs::read_to_string(d.legacy()).unwrap(), old);
        assert_eq!(
            Catalog::load(&d.path(), &d.legacy()).unwrap().get("mesh:1"),
            Some(&"Door".to_owned())
        );
    }
    #[test]
    fn rejects_stale_writes_and_keeps_last_good_backup() {
        let d = Scratch::new();
        let mut c = Catalog::default();
        c.records.insert("mesh:1".into(), record("Door"));
        c.save(&d.path()).unwrap();
        let mut stale = c.clone();
        let old = fs::read_to_string(d.path()).unwrap();
        c.records.insert("mesh:1".into(), record("Gate"));
        c.save(&d.path()).unwrap();
        assert_eq!(fs::read_to_string(sidecar(&d.path(), ".bak")).unwrap(), old);
        stale.records.clear();
        assert!(stale.save(&d.path()).is_err());
        assert_eq!(
            Catalog::load(&d.path(), &d.legacy()).unwrap().get("mesh:1"),
            Some(&"Gate".to_owned())
        );
    }
    #[test]
    fn invalid_edit_does_not_change_disk() {
        let d = Scratch::new();
        let mut c = Catalog::default();
        c.save(&d.path()).unwrap();
        let old = fs::read_to_string(d.path()).unwrap();
        c.records.insert(
            "x".into(),
            Record {
                confidence: Confidence::Verified,
                ..record("Door")
            },
        );
        assert!(c.save(&d.path()).is_err());
        assert_eq!(fs::read_to_string(d.path()).unwrap(), old);
    }
    #[test]
    fn resource_and_instance_names_are_independent() {
        let mut c = Catalog::default();
        c.records.insert("mesh:1".into(), record("Door"));
        c.records.insert("placed:3".into(), record("East entrance"));
        c.records.remove("placed:3");
        assert_eq!(c.get("mesh:1"), Some(&"Door".into()));
    }
    #[test]
    fn failed_temp_write_preserves_catalog_and_releases_lock() {
        let d = Scratch::new();
        let mut c = Catalog::default();
        c.save(&d.path()).unwrap();
        let old = fs::read_to_string(d.path()).unwrap();
        fs::create_dir(sidecar(&d.path(), ".pending")).unwrap();
        c.records.insert("x".into(), record("Door"));
        assert!(c.save(&d.path()).is_err());
        assert_eq!(fs::read_to_string(d.path()).unwrap(), old);
        assert!(!sidecar(&d.path(), ".lock").exists());
    }
    #[test]
    fn malformed_legacy_is_not_silently_discarded() {
        let d = Scratch::new();
        fs::write(d.legacy(), "bad header\na\tb").unwrap();
        assert!(Catalog::load(&d.path(), &d.legacy()).is_err());
        assert!(!d.path().exists());
    }
}
