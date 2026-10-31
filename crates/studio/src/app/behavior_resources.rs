//! Evidence inventory, not a native behavior writer or a control-flow closure proof.
use super::{collision_recipes::digest, form_database::{BaseForm, FormKey}};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Classification { Static, AnimatedOnly, ScriptedOnly, AnimatedAndScripted, UnknownDependencies }
impl Classification {
    pub fn label(self) -> &'static str { match self {
        Self::Static => "Static", Self::AnimatedOnly => "Animated Only",
        Self::ScriptedOnly => "Scripted Only", Self::AnimatedAndScripted => "Animated + Scripted",
        Self::UnknownDependencies => "Unknown Dependencies",
    }}
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Role { ManRecord, Anm, ActorBinding, MotionStream, Trigger, Dialogue, Condition, ScriptContext, ModelSelection }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Evidence {
    /// Logical carrier identity and exact decoded carrier hash; offsets are decoded bytes.
    pub source: String,
    pub sha256: String,
    pub offset: usize,
    pub length: usize,
    pub consumer: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Edge {
    pub role: Role,
    pub target: String,
    pub form: Option<FormKey>,
    pub evidence: Evidence,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Dependencies {
    pub edges: Vec<Edge>,
    /// Explicit scope reviews, never inferred from an empty scan or an asset category.
    pub animation_review: Option<Evidence>,
    pub scripting_review: Option<Evidence>,
    pub unresolved: Vec<String>,
}
impl Dependencies {
    pub fn validate(&self) -> Result<()> {
        for e in self.edges.iter().map(|e| &e.evidence).chain(self.animation_review.iter()).chain(self.scripting_review.iter()) {
            ensure!(!e.source.trim().is_empty() && !e.consumer.trim().is_empty(), "Dependency evidence lacks source/consumer");
            ensure!(e.sha256.len()==64 && e.sha256.bytes().all(|b| b.is_ascii_hexdigit()), "Dependency evidence lacks SHA256");
            ensure!(e.length>0 && e.offset.checked_add(e.length).is_some(), "Invalid dependency evidence span");
        }
        ensure!(self.edges.iter().all(|e| !e.target.trim().is_empty()), "Empty dependency target");
        Ok(())
    }
    pub fn classification(&self) -> Classification {
        if self.validate().is_err() || !self.unresolved.is_empty() || self.animation_review.is_none() || self.scripting_review.is_none() {
            return Classification::UnknownDependencies;
        }
        let animated=self.edges.iter().any(|e| matches!(e.role,Role::Anm));
        let scripted=self.edges.iter().any(|e| matches!(e.role,Role::ManRecord|Role::MotionStream|Role::Trigger|Role::Dialogue|Role::Condition|Role::ScriptContext));
        match (animated,scripted) {(false,false)=>Classification::Static,(true,false)=>Classification::AnimatedOnly,(false,true)=>Classification::ScriptedOnly,(true,true)=>Classification::AnimatedAndScripted}
    }
    fn add(&mut self, role: Role, target: String, source: &str, bytes: &[u8], offset: usize, length: usize, consumer: &str) {
        self.edges.push(Edge {role,target,form:None,evidence:Evidence {source:source.into(),sha256:digest(bytes),offset,length,consumer:consumer.into()}});
    }
    /// Partition-1 actor index, NOT flattened MAN index or model selector.
    pub fn from_man(source: &str, man: &[u8], actor: usize) -> Result<Self> {
        use legaia_scene_formats::{man_section,man_motion,field_disasm::{LinearWalker,InsnInfo,MenuCtrlKind}};
        let file=man_section::parse(man).map_err(|e| anyhow::anyhow!("{e}"))?;
        let (start,pc0,len)=legaia_engine_field::man_field_scripts::partition_record_span(&file,man,1,actor)
            .ok_or_else(||anyhow::anyhow!("MAN partition-1 actor record unavailable"))?;
        let mut out=Self::default();
        out.add(Role::ManRecord,format!("{source}:P1:{actor}"),source,man,start,len,"FUN_8003A1E4 / partition_record_span");
        out.add(Role::ActorBinding,format!("field-actor:+0x50={}",file.partitions[0].len()+actor),source,man,start,pc0,"FUN_8003A1E4");
        for decoded in LinearWalker::new(&man[start..start+len],pc0) {
            let insn=match decoded {Ok(i)=>i,Err((pc,e))=> {out.unresolved.push(format!("Field decode stopped at {}: {e:?}",start+pc));break;}};
            let role=match &insn.info {
                InsnInfo::TextSegment{..}|InsnInfo::Picker{..}|InsnInfo::MenuCtrl{kind:MenuCtrlKind::Nibble5Dialog{..},..}=>Some(Role::Dialogue),
                InsnInfo::BBoxTest{..}=>Some(Role::Trigger),
                InsnInfo::SystemFlag{..}|InsnInfo::LFlag{..}|InsnInfo::GFlag{..}|InsnInfo::CFlag{..}|InsnInfo::CondJmp{..}|InsnInfo::InventoryCmp{..}=>Some(Role::Condition),
                InsnInfo::SpawnRecord{..}=>Some(Role::ScriptContext),
                InsnInfo::Animate{..}=> {out.unresolved.push("Field ANIMATE block is not proof of an ANM resource binding".into());Some(Role::ModelSelection)},
                _=>None,
            };
            if let Some(role)=role {out.add(role,format!("{:?}",insn.info),source,man,start+insn.pc,insn.size,"field_disasm / native field VM");}
            if let Some(id)=insn.extended {out.add(Role::ScriptContext,format!("MAN:flat:{id}"),source,man,start+insn.pc,insn.size,"field VM cross-context dispatch");}
        }
        for rec in man_motion::motion_records(man,&file) {
            if !rec.bindings.iter().any(|b| usize::from(b.actor_id)==file.partitions[0].len()+actor && b.actor_id!=0xf8 && b.actor_id!=0xfb) {continue;}
            out.add(Role::MotionStream,format!("motion-record:{}",rec.index),source,man,rec.offset,rec.end_offset-rec.offset,"FUN_8003A9D4 / FUN_80038158");
            for variant in man_motion::stream_variants(man,&rec) {
                if let Some(flag)=variant.gate_flag() {out.add(Role::Condition,format!("system-flag:{flag}:set"),source,man,variant.header_offset,4,"FUN_80038158 variant selection");}
            }
        }
        out.unresolved.extend([
            "ANM/pose/equipment/model-bank binding and texture lifetime not closed".into(),
            "Incoming scripts, triggers, dialogue branches and story-state dependencies require control-flow closure".into(),
            "Native reference relocation, instance-local state and resource lifetime not implemented by this inventory".into(),
        ]);
        out.validate()?;
        Ok(out)
    }
    /// Resolve the native initial scene-bank animation selector. Special actor banks
    /// remain explicit gaps; model/pose compatibility is not inferred from parse success.
    pub fn bind_scene_animation(&mut self, source: &str, man: &[u8], actor: usize, scene: &legaia_engine_scene::scene::Scene) -> Result<()> {
        use legaia_scene_formats::man_section;
        use legaia_engine_scene::npc_catalog::{scene_anm_bundle,scene_anm_prot};
        let file=man_section::parse(man).map_err(|e|anyhow::anyhow!("{e}"))?;
        let placement=file.actor_placement(man,actor).ok_or_else(||anyhow::anyhow!("Actor header unavailable"))?;
        let header=placement.record_offset+1+placement.local_count*2;
        self.add(Role::ModelSelection,format!("model-bank:{}; animation-id:{}",placement.model_index,placement.anim_id),source,man,header,4,"FUN_8003A1E4 actor placement header");
        if placement.anim_id==0 {return Ok(());}
        if placement.special_model {
            self.unresolved.push("Special actor requires global model/locomotion bank adapter".into());return Ok(());
        }
        let bundle=scene_anm_bundle(scene).ok_or_else(||anyhow::anyhow!("Scene ANM bundle unresolved"))?;
        let prot=scene_anm_prot(scene).ok_or_else(||anyhow::anyhow!("ANM PROT carrier unresolved"))?;
        let record=usize::from(placement.anim_id)-1;
        bundle.record(record)?;
        let offset=*bundle.record_offsets.get(record).ok_or_else(||anyhow::anyhow!("ANM record offset unavailable"))? as usize;
        let length=bundle.record_bytes(record).len();
        self.add(Role::Anm,format!("prot:{prot}:ANM:record:{record}"),&format!("prot:{prot}:decoded-ANM"),&bundle.decoded,offset,length,"actor +0x5C = ANM record index + 1 / scene_anm_bundle");
        self.unresolved.push("Initial ANM record resolved; runtime clip switches, bone compatibility and relocation remain unverified".into());
        self.validate()
    }
    /// Validate ANM bytes independently; an explicit consumer binding is still required.
    pub fn attach_anm(&mut self, source: &str, bytes: &[u8], binding: Evidence) -> Result<()> {
        legaia_actor_formats::player_anm::parse(bytes)?;
        let mut draft=self.clone();
        draft.add(Role::Anm,format!("ANM:{}",digest(bytes)),source,bytes,0,bytes.len(),"player_anm::parse (format evidence only)");
        draft.edges.push(Edge {role:Role::ActorBinding,target:source.into(),form:None,evidence:binding});
        draft.validate()?;
        *self=draft;
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
pub(super) struct CopyAssessment {
    pub classification: Classification,
    pub requested_modes: [&'static str;3],
    pub static_visual: &'static str,
    pub animated: &'static str,
    pub behavior_preserving: &'static str,
    pub fully_functional: bool,
    pub unresolved: Vec<String>,
}
impl BaseForm {
    pub fn dependency_classification(&self) -> Classification {
        self.native_behavior.as_ref().map_or(Classification::UnknownDependencies,Dependencies::classification)
    }
    pub fn copy_assessment(&self) -> CopyAssessment {
        CopyAssessment {classification:self.dependency_classification(),requested_modes:["Static Visual Copy","Animated Copy","Behavior-Preserving Copy"],
            static_visual:"existing visual writer; consult per-operation material/registration results",
            animated:if self.kind==super::form_database::FormKind::Actor {"B2B detached field-actor compiler available; consult per-Reference registration/state results"}else{"not established: native animation relocation and per-reference binding unverified"},
            behavior_preserving:"not established: native behavior relocation and independent instance state unverified",
            fully_functional:false,unresolved:self.native_behavior.as_ref().map_or_else(||vec!["Native dependency inventory absent (legacy or undiscovered asset)".into()],|d|d.unresolved.clone())}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::form_database::{FileId,FormKind,FormDatabase,Transform,CommandHistory};
    fn evidence() -> Evidence {Evidence {source:"fixture:decoded".into(),sha256:digest(&[0x21]),offset:0,length:1,consumer:"test scope review".into()}}
    fn reviewed() -> Dependencies {Dependencies {animation_review:Some(evidence()),scripting_review:Some(evidence()),..Default::default()}}
    fn edge(role:Role)->Edge {Edge {role,target:"fixture:dependency".into(),form:None,evidence:evidence()}}
    #[test]
    fn classification_requires_both_scope_reviews_and_never_infers_static_from_category() {
        assert_eq!(Dependencies::default().classification(),Classification::UnknownDependencies);
        for (roles,expected) in [
            (vec![],Classification::Static),
            (vec![Role::Anm],Classification::AnimatedOnly),
            (vec![Role::Dialogue],Classification::ScriptedOnly),
            (vec![Role::Anm,Role::ManRecord],Classification::AnimatedAndScripted),
        ] {
            let mut d=reviewed();d.edges=roles.into_iter().map(edge).collect();
            assert_eq!(d.classification(),expected);
            d.unresolved.push("missing native consumer".into());
            assert_eq!(d.classification(),Classification::UnknownDependencies);
            d.unresolved.clear();d.animation_review=None;
            assert_eq!(d.classification(),Classification::UnknownDependencies);
        }
    }
    #[test]
    fn native_man_inventory_preserves_record_identity_and_decode_failure() {
        let man=crate::app::native_script_dependencies::fixture_man(&[
            &[0xa1,0,0x21], &[0x4c,0x73],
        ]);
        let a=Dependencies::from_man("fixture:MAN",&man,0).unwrap();
        assert!(a.edges.iter().any(|e|e.role==Role::ScriptContext && e.target=="MAN:flat:0"));
        for e in &a.edges {assert_eq!(e.evidence.sha256,digest(&man));assert!(e.evidence.offset+e.evidence.length<=man.len());}
        let b=Dependencies::from_man("fixture:MAN",&man,1).unwrap();
        assert!(b.unresolved.iter().any(|s|s.contains("decode stopped")));
        assert!(Dependencies::from_man("fixture:MAN",&man,2).is_err());
        assert_eq!(a.classification(),Classification::UnknownDependencies);
        assert_eq!(a.edges[0].target,"fixture:MAN:P1:0");
    }
    #[test]
    fn invalid_anm_or_binding_is_transactional() {
        let mut d=reviewed();let before=d.clone();
        assert!(d.attach_anm("fixture:ANM",&[0;12],evidence()).is_err());
        assert_eq!(d,before);
        let mut e=edge(Role::Dialogue);e.evidence.offset=usize::MAX;
        d.edges.push(e);assert!(d.validate().is_err());
        assert_eq!(d.classification(),Classification::UnknownDependencies);
    }
    #[test]
    fn dependency_forms_are_indexed_validated_and_survive_duplicate_undo_and_persistence() {
        let mut db=FormDatabase::new(FileId::new("retail").unwrap(),FileId::new("project").unwrap()).unwrap();
        let (base,cell,target)=db.transaction(|d|Ok((d.create_base("Actor".into(),FormKind::Actor)?,d.create_base("Cell".into(),FormKind::Cell)?,d.create_base("Script".into(),FormKind::Script)?))).unwrap();
        db.transaction(|d|d.edit_base(&base,|b|{let mut deps=reviewed();let mut e=edge(Role::ManRecord);e.form=Some(target.clone());deps.edges.push(e);b.native_behavior=Some(deps);Ok(())})).unwrap();
        assert!(db.use_info(&target).users.contains(&base));
        let mut history=CommandHistory::default();
        let r=history.execute(&mut db,|d|d.place_reference(base.clone(),cell,Transform::default())).unwrap();
        let dup=history.execute(&mut db,|d|d.duplicate_reference(&r)).unwrap();
        assert_ne!(r,dup);assert!(!db.resolve_reference(&dup).unwrap().base.copy_assessment().fully_functional);
        history.undo(&mut db).unwrap();history.redo(&mut db).unwrap();
        let mut loaded:FormDatabase=serde_json::from_slice(&serde_json::to_vec(&db).unwrap()).unwrap();loaded.validate().unwrap();
        assert_eq!(loaded.resolve_base(&base).unwrap().native_behavior,db.resolve_base(&base).unwrap().native_behavior);
        let before=serde_json::to_vec(&db).unwrap();
        let missing=FormKey::new(FileId::new("project").unwrap(),0xffff).unwrap();
        assert!(db.transaction(|d|d.edit_base(&base,|b|{b.native_behavior.as_mut().unwrap().edges[0].form=Some(missing);Ok(())})).is_err());
        assert_eq!(serde_json::to_vec(&db).unwrap(),before);
    }
    #[test]
    fn legacy_base_loads_as_unknown_without_hiding_asset_or_claiming_functional_copy() {
        let base=BaseForm::empty(FormKey::new(FileId::new("project").unwrap(),1).unwrap(),"Legacy".into(),FormKind::StaticEnvironment);
        let mut value=serde_json::to_value(&base).unwrap();value.as_object_mut().unwrap().remove("native_behavior");
        let loaded:BaseForm=serde_json::from_value(value).unwrap();
        assert!(loaded.kind.placeable());assert_eq!(loaded.dependency_classification(),Classification::UnknownDependencies);
        assert!(!loaded.copy_assessment().fully_functional);
    }
}

#[cfg(test)]
#[test]
#[ignore = "requires LCK_AUDIT_SOURCE retail image; read-only native dependency census"]
fn stage_b2a_native_dependency_census() -> Result<()> {
    let source=std::path::PathBuf::from(std::env::var("LCK_AUDIT_SOURCE")?);
    let library=crate::library::Library::open(&source)?;
    let mut names=library.index.cdname_scene_names();names.sort();names.dedup();
    let (mut scenes,mut actors,mut animations,mut edges,mut gaps)=(0,0,0,0,0);
    for name in names {
        let scene=match legaia_engine_scene::scene::Scene::load(&library.index,&name) {Ok(s)=>s,Err(_)=>continue};
        let Some(man)=scene.field_man_payload(&library.index)? else {continue};
        let file=match legaia_scene_formats::man_section::parse(&man) {Ok(f)=>f,Err(_)=>continue};
        scenes+=1;
        for placement in file.actor_placements(&man) {
            let mut deps=Dependencies::from_man(&format!("{name}:MAN"),&man,placement.index)?;
            if let Err(error)=deps.bind_scene_animation(&format!("{name}:MAN"),&man,placement.index,&scene) {deps.unresolved.push(format!("Initial animation: {error:#}"));}
            deps.validate()?;
            assert_eq!(deps.classification(),Classification::UnknownDependencies);
            for edge in &deps.edges {
                if edge.evidence.source.ends_with(":MAN") {
                    assert_eq!(edge.evidence.sha256,digest(&man));
                    assert!(edge.evidence.offset+edge.evidence.length<=man.len());
                }
            }
            animations+=deps.edges.iter().filter(|e|e.role==Role::Anm).count();
            edges+=deps.edges.len();gaps+=deps.unresolved.len();actors+=1;
            let restored:Dependencies=serde_json::from_slice(&serde_json::to_vec(&deps)?)?;
            assert_eq!(restored,deps);
        }
    }
    assert!(scenes>0 && actors>0 && animations>0);
    println!("B2A read-only census: {scenes} MAN scenes, {actors} actors, {animations} initial ANM bindings, {edges} observed edges, {gaps} explicit gaps; no behavior/gameplay certification");
    Ok(())
}
