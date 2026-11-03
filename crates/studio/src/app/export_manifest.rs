//! U1B.1 unified native edit manifest.
//!
//! Collects pending object, floor, and collision changes into one
//! immutable snapshot. Export routing is deliberately unchanged.
//!
//! This manifest is an audit input for future unified compilation,
//! NOT proof that every operation is serializable to retail PS1 MAP.

use std::collections::BTreeSet;
use std::path::PathBuf;

use super::{
    collision_edit,
    ground_edit_ops,
    scene_instances,
    world_view,
};

#[derive(Clone,Debug,serde::Serialize)]
pub(super) struct ObjectEdit {
    pub action:String,
    pub identity:String,
    pub serialized:String,
}

#[derive(Clone,Debug,serde::Serialize)]
pub(super) struct GroundEdit {
    pub action:String,
    pub source:[u8;2],
    pub copy_uid:Option<u64>,
    pub transform:Option<ground_edit_ops::Transform>,
}

#[derive(Clone,Debug,serde::Serialize)]
pub(super) struct WallEdit {
    pub x:i32,
    pub z:i32,
    pub occupied:bool,
}

#[derive(Clone,Debug,serde::Serialize)]
pub(super) struct VerticalPreview {
    pub x:i32,
    pub z:i32,
    pub offset:i32,
}

#[derive(Clone,Debug,serde::Serialize)]
pub(super) struct Manifest {
    pub version:u32,
    pub scene:String,
    pub objects:Vec<ObjectEdit>,
    pub ground:Vec<GroundEdit>,
    pub walls:Vec<WallEdit>,
    pub vertical_previews:Vec<VerticalPreview>,
    pub editor_metadata_records:usize,
    pub superseded_object_transforms:usize,
    pub notices:Vec<String>,
}

impl Manifest {
    pub(super) fn summary(&self)->String {
        format!(
            "Objects: {} | Ground: {} | Wall cells: {} | Vertical previews: {} | Superseded transforms: {}",
            self.objects.len(),
            self.ground.len(),
            self.walls.len(),
            self.vertical_previews.len(),
            self.superseded_object_transforms
        )
    }
}

pub(super) fn capture(
    scene:&str,
    world:&world_view::WorldModel,
    floors:&ground_edit_ops::State,
    collision:&collision_edit::State,
)->Manifest {
    let overlay=scene_instances::serialize(scene,world);
    let (walls,heights)=collision.native_edit_snapshot();
    from_parts(scene,&overlay,floors,walls,heights)
}

fn from_parts(
    scene:&str,
    overlay:&str,
    floors:&ground_edit_ops::State,
    walls:Vec<(i32,i32,bool)>,
    heights:Vec<(i32,i32,i32)>,
)->Manifest {
    let mut result=Manifest {
        version:1,
        scene:scene.into(),
        objects:Vec::new(),
        ground:Vec::new(),
        walls:Vec::new(),
        vertical_previews:Vec::new(),
        editor_metadata_records:0,
        superseded_object_transforms:0,
        notices:Vec::new(),
    };

    // Capture native deletions first so previous transforms of
    // those exact records can be marked as superseded.
    let deleted:BTreeSet<String>=overlay.lines().filter_map(|line|{
        let fields:Vec<&str>=line.split('\t').collect();
        if fields.len()>=3 && fields[0]=="HIDE" {
            Some(format!("{}:{}",fields[1],fields[2]))
        } else {
            None
        }
    }).collect();

    for line in overlay.lines() {
        let fields:Vec<&str>=line.split('\t').collect();
        let action=fields.first().copied().unwrap_or("");

        if action=="LCK_SCENE_V1" || action.is_empty() {
            continue;
        }

        if matches!(action,"NAME"|"NATIVE_NAME"|"INSTANCE_NAME") {
            result.editor_metadata_records+=1;
            continue;
        }

        let native_identity=if fields.len()>=3 &&
            matches!(action,"MOVE"|"ROTATE"|"SCALE"|"HIDE"|
                "NPC_MODEL"|"ANIM") {
            Some(format!("{}:{}",fields[1],fields[2]))
        } else {
            None
        };

        // Delete wins over an earlier transform. Preserve the
        // deletion itself, not the superseded operation.
        if matches!(action,"MOVE"|"ROTATE"|"SCALE") &&
            native_identity.as_ref().is_some_and(|id|deleted.contains(id)) {
            result.superseded_object_transforms+=1;
            continue;
        }

        let identity=native_identity.unwrap_or_else(||{
            fields.get(1).copied().unwrap_or("unidentified").into()
        });

        let known=matches!(action,
            "MOVE"|"ROTATE"|"SCALE"|"HIDE"|
            "INSTANCE"|"INSTANCE_ROT"|"INSTANCE_SCALE"|
            "INSTANCE_MODEL"|"INSTANCE_ANIM"|
            "FOREIGN"|"FOREIGN_STATIC"|
            "NPC_MODEL"|"ANIM"|
            "COLLISION"|"MESH_COLLISION"
        );

        if !known {
            result.notices.push(format!(
                "Unclassified scene record retained in manifest: {action}"
            ));
        }

        result.objects.push(ObjectEdit {
            action:action.into(),
            identity,
            serialized:line.into(),
        });
    }

    // Native ground changes are identified by source cell, not
    // by the transient viewport selection or render object ID.
    for (&cell,&transform) in &floors.originals {
        if floors.deleted.contains(&cell) {
            result.notices.push(format!(
                "Ground source ({},{}) was both transformed and deleted; deletion takes precedence in the manifest.",
                cell[0],cell[1]
            ));
            continue;
        }

        result.ground.push(GroundEdit {
            action:"transform".into(),
            source:cell,
            copy_uid:None,
            transform:Some(transform),
        });
    }

    for copy in &floors.copies {
        result.ground.push(GroundEdit {
            action:"copy".into(),
            source:copy.source,
            copy_uid:Some(copy.uid),
            transform:Some(copy.transform),
        });
    }

    for &cell in &floors.deleted {
        result.ground.push(GroundEdit {
            action:"delete".into(),
            source:cell,
            copy_uid:None,
            transform:None,
        });
    }

    // Wall changes remain independent of ground tiers.
    // A later writer must merge upper and lower MAP nibbles.
    for (x,z,occupied) in walls {
        result.walls.push(WallEdit{x,z,occupied});
    }

    for (x,z,offset) in heights {
        result.vertical_previews.push(
            VerticalPreview{x,z,offset}
        );
    }

    if !result.vertical_previews.is_empty() {
        result.notices.push(
            "Vertical collision offsets are editor-preview only; retail MAP cannot encode their heights.".into()
        );
    }

    if !floors.scene.is_empty() && floors.scene!=scene {
        result.notices.push(
            "Ground edit layer refers to another scene; do not compile until reconciled.".into()
        );
    }

    if !result.ground.is_empty() &&
        (!result.objects.is_empty() || !result.walls.is_empty()) {
        result.notices.push(
            "Mixed native floor serialization is NOT implemented by U1B.1; this snapshot is audit-only.".into()
        );
    }

    result
}

pub(super) fn write_report(
    manifest:&Manifest
)->std::io::Result<PathBuf> {
    let cleaned:String=manifest.scene.chars()
        .filter(|c|c.is_ascii_alphanumeric() || *c=='_' || *c=='-')
        .take(64)
        .collect();

    let name=if cleaned.is_empty() {
        "unknown".to_string()
    } else {
        cleaned
    };

    let path=std::env::temp_dir().join(
        format!("LCK_U1B1_manifest_{name}.json")
    );

    let bytes=serde_json::to_vec_pretty(manifest)
        .map_err(std::io::Error::other)?;

    std::fs::write(&path,bytes)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletion_supersedes_native_transforms() {
        let doc=concat!(
            "LCK_SCENE_V1\ttown01\n",
            "MOVE\tplacement\t12\t100\t0\t0\n",
            "ROTATE\tplacement\t12\t0\t1\t0\n",
            "HIDE\tplacement\t12\n",
            "MOVE\tplacement\t13\t50\t0\t0\n",
        );

        let floors=ground_edit_ops::State::default();
        let m=from_parts("town01",doc,&floors,vec![],vec![]);

        assert_eq!(m.superseded_object_transforms,2);
        assert_eq!(m.objects.len(),2);
        assert!(m.objects.iter().any(|o|o.action=="HIDE"));
        assert!(m.objects.iter().any(|o|
            o.action=="MOVE" && o.identity=="placement:13"
        ));
    }

    #[test]
    fn mixed_operations_are_all_accounted_for() {
        let doc=concat!(
            "LCK_SCENE_V1\ttown01\n",
            "HIDE\tplacement\t14\n",
            "INSTANCE\t99\tplacement\t15\t0\t0\t0\n",
            "NAME\tplacement\t15\t414243\n",
        );

        let mut floors=ground_edit_ops::State::default();
        floors.scene="town01".into();
        floors.deleted.insert([7,8]);
        floors.copies.push(ground_edit_ops::CopyPiece{
            uid:77,
            source:[9,10],
            transform:ground_edit_ops::Transform::default(),
        });

        let m=from_parts(
            "town01",doc,&floors,
            vec![(20,30,false),(21,30,true)],
            vec![],
        );

        assert_eq!(m.objects.len(),2);
        assert_eq!(m.ground.len(),2);
        assert_eq!(m.walls.len(),2);
        assert_eq!(m.editor_metadata_records,1);
        assert!(m.notices.iter().any(|n|
            n.contains("audit-only")
        ));
    }

    #[test]
    fn height_preview_is_not_a_native_wall_write() {
        let floors=ground_edit_ops::State::default();
        let m=from_parts(
            "town01","LCK_SCENE_V1\ttown01\n",
            &floors,vec![],vec![(4,5,120)],
        );

        assert!(m.walls.is_empty());
        assert_eq!(m.vertical_previews.len(),1);
        assert!(m.notices.iter().any(|n|
            n.contains("preview only")
        ));
    }

    #[test]
    fn ground_delete_takes_precedence_over_transform() {
        let mut floors=ground_edit_ops::State::default();
        floors.deleted.insert([5,6]);
        floors.originals.insert(
            [5,6],
            ground_edit_ops::Transform::default(),
        );

        let m=from_parts(
            "town01","LCK_SCENE_V1\ttown01\n",
            &floors,vec![],vec![],
        );

        assert_eq!(m.ground.len(),1);
        assert_eq!(m.ground[0].action,"delete");
        assert!(m.notices.iter().any(|n|
            n.contains("takes precedence")
        ));
    }
}
