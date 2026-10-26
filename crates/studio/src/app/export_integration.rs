//! Export Integration V1: deterministic, unified scene-change inventory.
//!
//! This module does NOT bypass PS1 encoding or collision preflight. Instead it
//! gives every scene change a traceable classification and writes a local log
//! of the native compilation attempt. Subsequent native TMD/TIM, MAP, MAN,
//! walk-floor, and collider writers can consume the same inventory.
use std::{fs, path::{Path, PathBuf}, time::{SystemTime, UNIX_EPOCH}};

#[derive(Debug, Default, PartialEq, Eq)]
struct ChangeCounts {
    labels: usize,
    native_moves: usize,
    native_copies: usize,
    foreign_assets: usize,
    native_rotations: usize,
    native_scales: usize,
    hidden: usize,
    npc_changes: usize,
    other: usize,
}

fn count_changes(scene_overlay: &str) -> ChangeCounts {
    let mut counts=ChangeCounts::default();
    for row in scene_overlay.lines().skip(1) {
        let op=row.split('\t').next().unwrap_or("");
        match op {
            "NAME" | "INSTANCE_NAME" => counts.labels += 1,
            "MOVE" => counts.native_moves += 1,
            "INSTANCE" => counts.native_copies += 1,
            "FOREIGN" => counts.foreign_assets += 1,
            "ROTATE" | "INSTANCE_ROT" => counts.native_rotations += 1,
            "SCALE" | "INSTANCE_SCALE" => counts.native_scales += 1,
            "HIDE" => counts.hidden += 1,
            "ANIM" | "INSTANCE_ANIM" | "NPC_MODEL" | "INSTANCE_MODEL" => counts.npc_changes += 1,
            "" => {},
            _ => counts.other += 1,
        }
    }
    counts
}

/// Can this scene contain a native change (as distinct from an editor label)?
/// The native compiler still makes the final, authoritative decision.
pub(super) fn only_editor_labels(overlay: &str) -> bool {
    let c=count_changes(overlay);
    c.labels>0 && c.native_moves==0 && c.native_copies==0 &&
    c.foreign_assets==0 && c.native_rotations==0 && c.native_scales==0 &&
    c.hidden==0 && c.npc_changes==0 && c.other==0
}

pub(super) fn render_report(cell: &str, overlay: &str, opened_image: Option<&Path>) -> String {
    let c=count_changes(overlay);
    let mut report=format!(
        "LCK EXPORT INTEGRATION V1 - CHANGE INVENTORY\n\
         Destination cell: {cell}\n\
         Source image: {}\n\
         Editor labels (not PS1 resource edits): {}\n\
         Original placements moved: {}\n\
         Same-cell instances duplicated: {}\n\
         Foreign Game-wide assets imported: {}\n\
         Object rotations: {}\n\
         Object non-default scales: {}\n\
         Hidden originals: {}\n\
         NPC/animation directives: {}\n\
         Other directives: {}\n\n",
        opened_image.map(|p|p.display().to_string()).unwrap_or_else(||"unavailable".to_string()),
        c.labels,c.native_moves,c.native_copies,c.foreign_assets,c.native_rotations,
        c.native_scales,c.hidden,c.npc_changes,c.other);
    report.push_str(
        "V1 compiler status:\n\
         - MAP moves and same-cell copies: routed to the existing native writer.\n\
         - Foreign assets requiring a NEW TMD/TIM, terrain, resizing, and transformed\n\
           collision: writers NOT implemented by this version.\n\
         - NPCs: existing MAN writer capabilities and restrictions remain.\n\
         - Editor labels: not game engine object names; they do not block\n\
           other native edits. Save .lckscene to persist labels.\n\
         - Failures never produce a partial or falsely successful IMG.\n\n");
    report.push_str("Full scene edit manifest (LCK_SCENE_V1):\n");
    report.push_str(overlay);
    report.push('\n');
    report
}

/// Write one unique, local, human-readable log for each attempt. Any I/O error
/// is returned to the caller as a diagnostic string (never a panic).
pub(super) fn write_log(scene: &str, report: &str, outcome: &str) -> Result<PathBuf, String> {
    let root=std::env::current_dir().unwrap_or_else(|_|std::env::temp_dir());
    let folder=root.join("export integrationV1_logs");
    fs::create_dir_all(&folder).map_err(|e|format!("Cannot create log folder {}: {e}",folder.display()))?;
    let tick=SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|e|format!("System clock error: {e}"))?.as_nanos();
    let safe=scene.chars().filter(|c|c.is_ascii_alphanumeric() || *c=='_' || *c=='-')
        .take(40).collect::<String>();
    let name=format!("{}_{}.txt",if safe.is_empty(){"scene"}else{&safe},tick);
    let file=folder.join(name);
    fs::write(&file,format!("{report}\nRESULT\n{outcome}\n"))
        .map_err(|e|format!("Cannot write {}: {e}",file.display()))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn labels_are_editor_metadata_not_native_edits() {
        assert!(only_editor_labels("LCK_SCENE_V1\ttown01\nNAME\tplacement\t5\t426f62\n"));
        assert!(!only_editor_labels("LCK_SCENE_V1\ttown01\nNAME\tplacement\t5\t426f62\nMOVE\tplacement\t5\t1\t2\t3\n"));
    }
    #[test]
    fn all_native_edit_kinds_are_explicit() {
        let c=count_changes("LCK_SCENE_V1\ttown01\nFOREIGN\t1\nINSTANCE\t2\nINSTANCE_ROT\t2\t0\t1\t0\nINSTANCE_SCALE\t2\t1\t2\t1\nHIDE\tplacement\t0\nINSTANCE_ANIM\t2\t0\t0\n");
        assert_eq!(c.foreign_assets,1);
        assert_eq!(c.native_copies,1);
        assert_eq!(c.native_rotations,1);
        assert_eq!(c.native_scales,1);
        assert_eq!(c.hidden,1);
        assert_eq!(c.npc_changes,1);
        assert_eq!(c.other,0);
    }
}
