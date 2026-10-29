//! PS1-native scene geometry assembled for the Studio CPU rasterizer.
//! TMD positions, walk-ground vertex elevations, per-cell texture coordinates,
//! CLUT palettes and tpages all come from the original game scene records.
//! The editor samples those ground tiles from assembled scene VRAM.

use std::collections::HashMap;
use std::sync::Arc;
use legaia_tim::Vram;
use legaia_engine_scene::scene_assembly::AssembledScene;
use crate::raster::Model;

mod npcs;
pub(super) use npcs::NpcInfo;
/// The MAN partition-1 actor index space, kept separate from MAP environment
/// indices and editor-created instances. Native MAN numbering begins at 1.
pub(super) const NPC_KEY_BASE: usize = 1_000_000_000;


#[derive(Clone)]
pub(super) struct WorldModel {
    pub naming:super::naming::Inventory,
    pub collision: super::collision_recipes::CollisionState,
    pub model: Option<Model>,
    pub instances: usize,
    pub unavailable: usize,
    pub unique_models: usize,
    pub ground_triangles: usize,
    /// Native textured ground triangles whose referenced TIM data is in VRAM.
    pub ground_vram_covered: usize,
    /// All triangles flagged textured from the MAP's native UV/page/CLUT data.
    pub ground_textured: usize,
    pub draws: Vec<DrawInfo>,
    /// Native scene MAN actor placement inventory (including conditionals).
    pub npcs: Vec<NpcInfo>,
    /// Recoverable catalog failure must not prevent environmental viewing.
    pub npc_issue: Option<String>,
    /// Source provenance for editor-only foreign-scene geometry. Never native MAP slots.
    pub foreign: Vec<ForeignRef>,
    /// Editor renderer only: original donor VRAM by draw ID. No PS1 disc mutation.
    pub foreign_vram: HashMap<u32, Arc<Vram>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ForeignRef {
    pub uid: u64,
    /// Explicitly copy the visible raw architecture; no donor MAN/ANM behavior.
    pub static_architecture: bool,
    pub source: String,
    pub kind: String,
    pub record: usize,
    /// Original donor PS1 model bytes owned by this LCK preview instance.
    /// Saving .lckscene still references the disc and restores these on load.
    pub raw_tmd: Vec<u8>,
    /// Source MAP object record, when the native placement is verifiable.
    pub native_record: Option<[u8; 32]>,
    /// Original wall bit positions inferred from the donor's MAP grid.
    /// These may include adjacent authored terrain bits: not proof of ownership.
    pub donor_wall_bits: Vec<(i32, i32)>,
    pub donor_center: [f32; 3],
}

#[derive(Clone)]
pub(super) struct DrawInfo {
    pub id: u32,
    pub terrain: bool,
    pub index: usize,
    pub center: [f32; 3],
    pub radius: f32,
    /// Original viewer-space center and vertex range for precise, reversible placement edits.
    pub original_center: [f32; 3],
    pub offset: [f32; 3],
    /// Additional editor-side Euler rotation in radians, X (pitch), Y (yaw), Z (roll).
    /// The native PS1 placement orientation is already baked into model.rest.
    pub rotation: [f32; 3],
    /// Editor-only scale around the unchanged native center (1.0 = original).
    pub scale: [f32; 3],
    pub vertex_start: usize,
    pub vertex_end: usize,
    /// None for a retail scene record; Some(native key) for an editor-created copy.
    pub source_ref: Option<(bool, usize)>,
    /// Stable editor-side UID for newly created copies. Never a PS1 identifier.
    pub editor_uid: Option<u64>,
    /// User-defined name for this scene instance, never a guessed asset identity.
    pub display_name: Option<String>,
    /// Native objects are non-destructively hidden. Created objects are removed.
    pub hidden: bool,
    /// Reversible original triangle indices; storing only affected triangles.
    pub hidden_triangles: Vec<(usize, [u32; 3])>,
}

/// Apply editor-only rotation around the object's own world-space center.
/// Native orientation is already baked into `rest`, so zero angles restore it.
pub(super) fn editor_rotate_vertex(rest: [f32; 3], pivot: [f32; 3], offset: [f32; 3],
                        rotation: [f32; 3], scale: [f32; 3]) -> [f32; 3] {
    let (sx,cx)=rotation[0].sin_cos();
    let (sy,cy)=rotation[1].sin_cos();
    let (sz,cz)=rotation[2].sin_cos();
    let p=[0,1,2].map(|k|(rest[k]-pivot[k])*scale[k]);
    // X then Y then Z; orthogonal rotations keep length and scene bounds intact.
    let x=p[0];
    let y=p[1]*cx-p[2]*sx;
    let z=p[1]*sx+p[2]*cx;
    let xx=x*cy+z*sy;
    let yy=y;
    let zz=-x*sy+z*cy;
    let rotated=[xx*cz-yy*sz,xx*sz+yy*cz,zz];
    [0,1,2].map(|k|pivot[k]+offset[k]+rotated[k])
}

impl WorldModel {
    /// Editor-only transform: leaves the original TMD, walk-ground, native
    /// placement records and loaded disc image unchanged. Rest coordinates
    /// are immutable, so repeated drags never introduce cumulative drift.
    pub fn set_offset(&mut self, terrain: bool, index: usize, offset: [f32; 3]) -> bool {
        if !offset.iter().all(|v| v.is_finite() && v.abs() < 1_000_000.0) { return false; }
        let Some(draw) = self.draws.iter_mut().find(|d| d.terrain == terrain && d.index == index) else { return false; };
        let Some(model) = self.model.as_mut() else { return false; };
        if draw.vertex_end > model.positions.len() || draw.vertex_end > model.rest.len() { return false; }
        for i in draw.vertex_start..draw.vertex_end {
            model.positions[i] = editor_rotate_vertex(model.rest[i], draw.original_center,
                offset, draw.rotation, draw.scale);
        }
        draw.offset = offset;
        draw.center = [0, 1, 2].map(|axis| draw.original_center[axis] + offset[axis]);
        // Deliberately DON'T refit the whole world here: that would cause
        // the camera to shift as the selected object is moved.
        true
    }

    /// Rotate only this draw's vertices about its original world-space center.
    /// Never changes the underlying native mesh/rest coordinates or PS1 files.
    pub fn set_rotation(&mut self, terrain: bool, index: usize, rotation: [f32; 3]) -> bool {
        if !rotation.iter().all(|v| v.is_finite() && v.abs() < 1000.0) { return false; }
        let Some(draw) = self.draws.iter_mut().find(|d| d.terrain == terrain && d.index == index)
            else { return false; };
        let Some(model) = self.model.as_mut() else { return false; };
        if draw.hidden || draw.vertex_start >= draw.vertex_end ||
            draw.vertex_end > model.positions.len() || draw.vertex_end > model.rest.len() {
            return false;
        }
        for i in draw.vertex_start..draw.vertex_end {
            model.positions[i] = editor_rotate_vertex(model.rest[i], draw.original_center,
                draw.offset, rotation, draw.scale);
        }
        draw.rotation = rotation;
        true
    }

    /// Per-axis object scaling in the editor. 1.0 is exactly the native
    /// geometry; 0.05..20 avoids degenerate triangles and severe overflows.
    /// Reconstruct from immutable rest coordinates: no cumulative drift.
    pub fn set_scale(&mut self, terrain: bool, index: usize, scale: [f32;3]) -> bool {
        if !scale.iter().all(|v| v.is_finite() && (0.05..=20.0).contains(v)) {
            return false;
        }
        let Some(draw) = self.draws.iter_mut()
            .find(|d| d.terrain == terrain && d.index == index) else { return false; };
        let Some(model) = self.model.as_mut() else { return false; };
        if draw.hidden || draw.vertex_start >= draw.vertex_end ||
            draw.vertex_end > model.positions.len() || draw.vertex_end > model.rest.len() {
            return false;
        }
        for i in draw.vertex_start..draw.vertex_end {
            model.positions[i] = editor_rotate_vertex(model.rest[i], draw.original_center,
                draw.offset, draw.rotation, scale);
        }
        draw.scale = scale;
        true
    }

    /// Editor-only identity allocation. MAN actor copies MUST use the same
    /// NPC_KEY_BASE namespace as the native MAN actors; otherwise the actor
    /// inspector, .lckscene serializer and PS1 MAN exporter cannot resolve the
    /// copy's model and animation. MAP objects retain their original namespace.
    fn next_copy_key(&self, npc: bool, first_created_index: usize) -> Result<usize,String> {
        if npc {
            let last = self.draws.iter()
                .filter(|d| !d.terrain && d.index >= NPC_KEY_BASE)
                .map(|d| d.index).max().unwrap_or(NPC_KEY_BASE - 1);
            last.checked_add(1).ok_or_else(||"No remaining NPC editor keys".into())
        } else {
            Ok(self.draws.iter()
                .filter(|d| !d.terrain && d.index < NPC_KEY_BASE)
                .map(|d| d.index)
                .max().and_then(|i| i.checked_add(1))
                .unwrap_or(first_created_index).max(first_created_index))
        }
    }

    /// Duplicate one real native draw as an editor-only instance.
    /// The source's native geometry, resource number and original PS1 data remain untouched.
    pub fn duplicate_native(&mut self, source_key: (bool, usize),
                            first_created_index: usize, uid: u64,
                            offset: [f32; 3]) -> Result<(bool, usize), String> {
        if uid == 0 || self.draws.iter().any(|d| d.editor_uid == Some(uid)) {
            return Err("Duplicate or zero editor UID".into());
        }
        if !offset.iter().all(|v| v.is_finite() && v.abs() < 1_000_000.0) {
            return Err("Invalid editor-only transform".into());
        }
        let npc_source=self.npc(source_key).cloned();
        if npc_source.as_ref().is_some_and(|n|n.is_copy || n.conditional || !n.drawable) {
            return Err("Cannot duplicate an unavailable or editor-created NPC source".into());
        }
        let source = self.at(source_key.0, source_key.1)
            .ok_or("Source native mesh was not assembled")?;
        let foreign=source.editor_uid.and_then(|uid|self.foreign.iter().find(|f|f.uid==uid)).cloned();
        if source.editor_uid.is_some() && foreign.is_none() { return Err("Source must be an original native record or actual foreign instance".into()); }
        let donor_vram=self.foreign_vram.get(&source.id).cloned();
        let source_start = source.vertex_start;
        let source_end = source.vertex_end;
        let source_center = source.original_center;
        let source_radius = source.radius;
        let source_name=source.display_name.clone();
        // MAN actors and MAP environmental objects MUST NOT share editor-key
        // namespaces. The old allocator always returned an environment index;
        // clones retained NPC pixels but WorldModel::npc() could not see their
        // appearance metadata (NPC copy has no appearance record at export).
        // Keep the real MAN key-space independent, including after scene reload.
        let new_index = self.next_copy_key(npc_source.is_some(), first_created_index)?;
        if self.at(false, new_index).is_some() { return Err("Instance index collision".into()); }
        let new_id = u32::try_from(self.draws.len()).map_err(|_| "Too many draw IDs")?;
        let model = self.model.as_mut().ok_or("Scene has no rendered model")?;
        let original_count = model.positions.len();
        if source_start >= source_end || source_end > original_count ||
            [model.rest.len(), model.uvs.len(), model.cba_tsb.len(),
             model.modulation.len(), model.fill.len(), model.textured.len(),
             model.object_ids.len()].iter().any(|&length| length != original_count) {
            return Err("Source geometry or vertex attributes are incomplete".into());
        }
        let added = source_end - source_start;
        let new_end = original_count.checked_add(added).ok_or("Too many vertices")?;
        if new_end > u32::MAX as usize { return Err("Native preview vertex limit exceeded".into()); }
        let triangles: Vec<[u32; 3]> = model.indices.chunks_exact(3).filter_map(|tri| {
            let points = [tri[0], tri[1], tri[2]];
            points.iter().all(|&i| (i as usize) >= source_start && (i as usize) < source_end)
                .then_some(points)
        }).collect();
        if triangles.is_empty() { return Err("Native source has no independent triangles".into()); }
        let rest = model.rest[source_start..source_end].to_vec();
        model.positions.extend(rest.iter().map(|p| [0,1,2].map(|k| p[k] + offset[k])));
        model.rest.extend_from_slice(&rest);
        model.uvs.extend_from_within(source_start..source_end);
        model.cba_tsb.extend_from_within(source_start..source_end);
        model.modulation.extend_from_within(source_start..source_end);
        model.fill.extend_from_within(source_start..source_end);
        model.textured.extend_from_within(source_start..source_end);
        model.object_ids.extend(std::iter::repeat_n(new_id, added));
        for tri in triangles {
            model.indices.extend(tri.map(|i| (i as usize - source_start + original_count) as u32));
        }
        let collision_owner=self.collision.owner(self,source_key);
        self.draws.push(DrawInfo {
            id: new_id, terrain: false, index: new_index,
            center: [0,1,2].map(|k| source_center[k] + offset[k]),
            radius: source_radius, original_center: source_center,
            offset, rotation: [0.0;3], scale: [1.0;3], vertex_start: original_count, vertex_end: new_end,
            source_ref: Some(source_key), editor_uid: Some(uid),
            display_name: source_name, hidden: false, hidden_triangles: Vec::new(),
        });
        self.instances += 1;
        if let Some(owner)=collision_owner {self.collision.duplicate(&owner,uid);}
        if let Some(mut donor)=foreign {
            let source_uid=donor.uid;donor.uid=uid;self.foreign.push(donor);
            if let Some(vram)=donor_vram {self.foreign_vram.insert(new_id,vram);}
            if let Some(source)=self.collision.foreign_sources.get(&source_uid).cloned(){self.collision.foreign_sources.insert(uid,source);}
            if let Some(mut recipe)=self.collision.mesh_recipes.get(&source_uid).cloned(){recipe.uid=uid;self.collision.mesh_recipes.insert(uid,recipe);}
            if let Some(name)=self.naming.foreign.get(&source_uid).cloned(){self.naming.foreign.insert(uid,name);}
            if let Some(name)=self.naming.foreign_legacy.get(&source_uid).cloned(){self.naming.foreign_legacy.insert(uid,name);}
            self.draws.last_mut().unwrap().source_ref=Some((false,usize::MAX));
        }else if npc_source.is_none() {
            if let Err(error)=self.ensure_automatic_collision((false,new_index)) {
                self.delete_created((false,new_index))?;
                return Err(error);
            }
        }
        if let Some(mut actor)=npc_source {
            actor.key=new_index;
            actor.is_copy=true;
            self.npcs.push(actor);
        }
        if let Some(recipe)=self.collision.recipes.iter().find(|a|
            a.owner==super::collision_recipes::ObjectId::Created(uid)) {
            let scene=recipe.recipe.source.object.scene.clone();
            if super::scene_instances::serialize(&scene,self).len()>2_000_000 {
                self.delete_created((false,new_index))?;
                return Err("Collision copy would exceed scene file safety limit".into());
            }
        }
        Ok((false, new_index))
    }


    /// Add an editor-only copy of ORIGINAL geometry from a different retail scene.
    /// This is a visual staging instance, NOT a native TMD/TIM/MAN import.
    /// Original donor TIMs are sampled from donor VRAM only inside Studio,
    /// so the destination PS1 scene and its own VRAM remain unchanged.
    pub fn insert_foreign(&mut self, donor:&AssembledScene, kind:&str, record:usize,
        uid:u64, center:[f32;3],
        source_index:&legaia_engine_scene::scene::ProtIndex) -> Result<(bool,usize),String> {
        if uid==0 || self.draws.iter().any(|d|d.editor_uid==Some(uid)) {
            return Err("Duplicate or invalid foreign instance UID".into());
        }
        if center.iter().any(|v|!v.is_finite() || v.abs()>=1_000_000.0) {
            return Err("Invalid foreign placement coordinates".into());
        }
        if self.foreign.len()>=512 { return Err("Foreign preview limit is 512 instances per cell".into()); }
        let (mesh, raw_points)=foreign_original_geometry(donor,kind,record)?;
        if mesh.positions.is_empty() || mesh.indices.is_empty() || mesh.positions.len()>250_000 {
            return Err("Donor has no usable geometry or exceeds preview budget".into());
        }
        let (original_center,radius)=bounds(&raw_points);
        // Q3 already copies the original UVs, CLUTs and donor VRAM.
        // Q6 adds retained source TMD and native MAP provenance without
        // interfering with that working rendering path.
        let source_slot = match kind {
            "placement" => donor.placements.get(record).map(|p|p.res_tmd),
            "terrain" => donor.terrain.get(record).map(|p|p.res_tmd),
            "mesh" => donor.env_tmds.get(record).copied(),
            _ => None,
        }.ok_or("Could not identify the donor source TMD slot")?;
        let raw_tmd = donor.res.tmds.get(source_slot)
            .ok_or("Original source TMD is missing")?.raw.clone();
        let (native_record, donor_wall_bits) = super::ps1_export::capture_donor_native(
            source_index,donor,kind,record,&raw_points);
        let collision_source=if kind=="placement" {
            super::collision_recipes::source_inventory(donor,source_index).ok()
                .and_then(|mut state|state.sources.remove(&(false,record)))
        } else {None};
        let index=self.next_copy_key(false,donor.placements.len())?;
        let id=u32::try_from(self.draws.len()).map_err(|_|"Too many world draw IDs")?;
        let model=self.model.as_mut().ok_or("Scene is missing world model")?;
        let start=model.positions.len();
        let end=start.checked_add(raw_points.len()).ok_or("Foreign vertex count overflow")?;
        if end>u32::MAX as usize { return Err("Foreign vertex index limit exceeded".into()); }
        if mesh.indices.iter().any(|&i|i as usize>=raw_points.len()) {
            return Err("Invalid original TMD triangle indices".into());
        }
        if [mesh.uvs.len(),mesh.cba_tsb.len(),mesh.modulation.len(),mesh.fill.len(),
            mesh.textured.len()].iter().any(|&n|n!=raw_points.len()) {
            return Err("Foreign donor TMD has inconsistent per-vertex attributes".into());
        }
        let centered=raw_points.iter().map(|p|[0,1,2].map(|axis|p[axis]-original_center[axis]+center[axis]))
            .collect::<Vec<[f32;3]>>();
        model.positions.extend_from_slice(&centered);
        model.rest.extend_from_slice(&centered);
        model.indices.extend(mesh.indices.iter().map(|&i|i+start as u32));
        model.object_ids.extend(std::iter::repeat_n(id,centered.len()));
        // Preserve real original UVs and palette/page records. During editor rendering,
        // this draw samples DONOR VRAM, never the destination cell's atlas.
        model.uvs.extend_from_slice(&mesh.uvs);
        model.cba_tsb.extend_from_slice(&mesh.cba_tsb);
        model.modulation.extend_from_slice(&mesh.modulation);
        model.fill.extend_from_slice(&mesh.fill);
        model.textured.extend_from_slice(&mesh.textured);
        self.draws.push(DrawInfo {id,terrain:false,index,center,radius,original_center:center,
            offset:[0.0;3],rotation:[0.0;3],scale:[1.0;3],vertex_start:start,vertex_end:end,
            source_ref:Some((false,usize::MAX)),editor_uid:Some(uid),display_name:None,
            hidden:false,hidden_triangles:Vec::new()});
        // Share one scene VRAM allocation among every copy imported from the same
        // original source; saving the overlay still references native asset IDs.
        let source_texture=self.foreign.iter().find(|entry| entry.source==donor.name)
            .and_then(|entry|self.draws.iter().find(|d|d.editor_uid==Some(entry.uid)))
            .and_then(|draw|self.foreign_vram.get(&draw.id)).cloned()
            .unwrap_or_else(||Arc::new(donor.res.vram.clone()));
        self.foreign_vram.insert(id,source_texture);
        self.foreign.push(ForeignRef{
            uid,static_architecture:true,source:donor.name.clone(),kind:kind.into(),record,
            raw_tmd,native_record,donor_wall_bits,donor_center:original_center
        });
        self.instances+=1;
        let legacy_item=match kind {"placement"=>Some(super::EditorCatalogItem::Draw(false,record)),"terrain"=>Some(super::EditorCatalogItem::Draw(true,record)),"mesh"=>Some(super::EditorCatalogItem::Mesh(record)),_=>None};
        if let Some(key)=legacy_item.and_then(|item|super::editor_item_name_key(Some(donor),item)) {self.naming.foreign_legacy.insert(uid,key);}
        if let Some(key)=super::naming::visual_key(&self.foreign.last().unwrap().raw_tmd,&super::naming::material_key(&donor.res.vram)) {
            self.naming.foreign.insert(uid,key);
        }
        if let Some(source)=collision_source {
            self.collision.foreign_sources.insert(uid,source);
            if let Err(error)=self.ensure_automatic_collision((false,index)) {
                self.delete_created((false,index))?;return Err(error);
            }
        }
        if let Err(error)=self.ensure_foreign_mesh_collision((false,index)) {
            self.delete_created((false,index))?;return Err(error);
        }
        Ok((false,index))
    }

    /// Rename a single placed instance, without changing the native asset catalog.
    pub fn set_display_name(&mut self, key: (bool, usize), name: Option<String>) -> bool {
        let Some(draw) = self.draws.iter_mut()
            .find(|d| (d.terrain, d.index) == key) else { return false; };
        draw.display_name = name;
        true
    }

    /// Non-destructive hide/unhide for authentic native objects. Degenerate
    /// triangles render nothing without renumbering unrelated vertex IDs,
    /// so camera navigation and selection continue to work unchanged.
    pub fn set_hidden(&mut self, key: (bool, usize), hide: bool) -> bool {
        let Some(draw) = self.draws.iter_mut()
            .find(|d| (d.terrain, d.index) == key) else { return false; };
        if draw.editor_uid.is_some() { return false; } // copies use delete_created
        if draw.hidden == hide { return true; }
        if draw.vertex_start == draw.vertex_end {
            draw.hidden = hide; // unrendered native record: hierarchy metadata only
            return true;
        }
        let Some(model) = self.model.as_mut() else { return false; };
        if hide {
            let (start, end) = (draw.vertex_start as u32, draw.vertex_end as u32);
            let mut stored = Vec::new();
            for (t, tri) in model.indices.chunks_exact_mut(3).enumerate() {
                let old = [tri[0], tri[1], tri[2]];
                if old.iter().all(|&i| start <= i && i < end) {
                    stored.push((3 * t, old));
                    tri[1] = tri[0]; tri[2] = tri[0];
                }
            }
            draw.hidden_triangles = stored;
        } else {
            for &(position, original) in &draw.hidden_triangles {
                if let Some(tri) = model.indices.get_mut(position..position + 3) {
                    tri.copy_from_slice(&original);
                } else { return false; }
            }
            draw.hidden_triangles.clear();
        }
        draw.hidden = hide;
        true
    }

    /// Completely remove an editor-created instance and compact its vertices,
    /// triangle indices and draw IDs. Never deletes or mutates native records.
    pub fn delete_created(&mut self, key: (bool, usize)) -> Result<(), String> {
        let pos = self.draws.iter().position(|d| (d.terrain, d.index) == key)
            .ok_or("Selected object no longer exists")?;
        if self.draws[pos].editor_uid.is_none() {
            return Err("This is a native object. Hide it in the editor instead".into());
        }
        let draw = &self.draws[pos];
        let (start, end, id) = (draw.vertex_start, draw.vertex_end, draw.id);
        let model = self.model.as_mut().ok_or("World geometry unavailable")?;
        if end > model.positions.len() || start >= end { return Err("Invalid instance vertex span".into()); }
        let cut = (end - start) as u32;
        model.indices = model.indices.chunks_exact(3).filter_map(|tri| {
            let item = [tri[0],tri[1],tri[2]];
            if item.iter().any(|&i| (i as usize) >= start && (i as usize) < end) { None }
            else { Some(item.map(|i| if i >= end as u32 {i-cut} else {i})) }
        }).flat_map(|tri| tri).collect();
        model.positions.drain(start..end);
        model.rest.drain(start..end);
        model.uvs.drain(start..end);
        model.cba_tsb.drain(start..end);
        model.modulation.drain(start..end);
        model.fill.drain(start..end);
        model.textured.drain(start..end);
        model.object_ids.drain(start..end);
        for value in &mut model.object_ids { if *value > id { *value -= 1; } }
        if let Some(uid)=self.draws[pos].editor_uid {
            self.foreign.retain(|f|f.uid!=uid);
            self.collision.recipes.retain(|a|a.owner!=super::collision_recipes::ObjectId::Created(uid));
            self.collision.foreign_sources.remove(&uid);
            self.collision.mesh_recipes.remove(&uid);
            self.naming.foreign.remove(&uid);
            self.naming.foreign_legacy.remove(&uid);
        }
        self.draws.remove(pos);
        self.foreign_vram.remove(&id);
        self.foreign_vram = std::mem::take(&mut self.foreign_vram).into_iter()
            .map(|(k,v)|(if k>id {k-1} else {k},v)).collect();
        self.npcs.retain(|n| n.key!=key.1 || key.0);
        for after in &mut self.draws {
            if after.id > id { after.id -= 1; }
            if after.vertex_start >= end {
                after.vertex_start -= cut as usize;
                after.vertex_end -= cut as usize;
            }
        }
        self.instances = self.instances.saturating_sub(1);
        Ok(())
    }

    /// Remove every editor-created copy; only appended vertices are removed.
    /// Used when reloading a saved editor project, never for native writeback.
    pub fn clear_created(&mut self) {
        self.collision.foreign_sources.clear();
        self.naming.foreign.clear();
        self.naming.foreign_legacy.clear();
        self.collision.mesh_recipes.clear();
        self.collision.recipes.retain(|a|matches!(a.owner,super::collision_recipes::ObjectId::Native(_)));
        self.foreign.clear();
        self.foreign_vram.clear();
        let Some(cut) = self.draws.iter().filter(|d| d.editor_uid.is_some())
            .map(|d| d.vertex_start).min() else { return };
        if let Some(model) = self.model.as_mut() {
            model.positions.truncate(cut);
            model.rest.truncate(cut);
            model.uvs.truncate(cut);
            model.cba_tsb.truncate(cut);
            model.modulation.truncate(cut);
            model.fill.truncate(cut);
            model.textured.truncate(cut);
            model.object_ids.truncate(cut);
            model.indices.retain(|&i| (i as usize) < cut);
        }
        let count = self.draws.iter().filter(|d| d.editor_uid.is_some()).count();
        self.draws.retain(|d| d.editor_uid.is_none());
        self.npcs.retain(|n|!n.is_copy);
        self.instances = self.instances.saturating_sub(count);
    }

    /// Optional center-to-center magnetic anchors; only when near a target.
    pub fn snap_nearby_anchor(&self, key: (bool, usize), desired: [f32; 3], radius: f32) -> [f32; 3] {
        let Some(item) = self.at(key.0, key.1) else { return desired };
        if !radius.is_finite() || radius <= 0.0 { return desired; }
        let center: [f32; 3] = [0, 1, 2].map(|k| item.original_center[k] + desired[k]);
        self.draws.iter().filter(|other| other.id != item.id && !other.hidden)
            .filter_map(|other| {
                let distance_sq: f32 = [0, 1, 2].iter().map(|&k| (center[k]-other.center[k]).powi(2)).sum();
                (distance_sq <= radius * radius).then_some((distance_sq, other.center))
            })
            .min_by(|a,b| a.0.total_cmp(&b.0))
            .map(|(_, target)| [0, 1, 2].map(|k| target[k] - item.original_center[k]))
            .unwrap_or(desired)
    }
}

/// Grid is anchored to each piece's ORIGINAL position, never world (0,0,0).
/// That preserves the original authentic placement when offset is zero.
pub(super) fn quantize_offset(delta: [f32; 3], size: f32) -> [f32; 3] {
    if !size.is_finite() || size <= 0.0 { return delta; }
    delta.map(|v| (v / size).round() * size)
}

#[cfg(test)]
mod transform_tests {
    use super::*;
    #[test]
    fn anchored_grid_retains_zero_and_snaps_relative_motion() {
        assert_eq!(quantize_offset([30.0, -31.0, 35.0], 32.0), [32.0, -32.0, 32.0]);
        assert_eq!(quantize_offset([15.0, 0.0, -15.0], 32.0), [0.0; 3]);
        assert_eq!(quantize_offset([1.5, -1.5, 0.0], 1.0), [2.0, -2.0, 0.0]);
    }
    #[test]
    fn duplicated_npcs_use_actor_namespace_and_leave_map_copy_keys_untouched() {
        let mut world = WorldModel {
            model: None, instances: 0, unavailable: 0, unique_models: 0,
            ground_triangles: 0, ground_vram_covered: 0, ground_textured: 0,
            npcs: Vec::new(), npc_issue: None,foreign:Vec::new(),naming:Default::default(),collision:Default::default(),foreign_vram:HashMap::new(),
            draws: vec![
                DrawInfo { id: 0, terrain: false, index: 18,
                    center:[0.0;3], radius:1.0, original_center:[0.0;3],
                    offset:[0.0;3], rotation:[0.0;3],scale:[1.0;3],
                    vertex_start:0,vertex_end:0, source_ref:None, editor_uid:None,
                    display_name:None,hidden:false,hidden_triangles:Vec::new() },
                DrawInfo { id: 1, terrain: false, index: NPC_KEY_BASE + 7,
                    center:[0.0;3], radius:1.0, original_center:[0.0;3],
                    offset:[0.0;3], rotation:[0.0;3],scale:[1.0;3],
                    vertex_start:0,vertex_end:0, source_ref:None, editor_uid:None,
                    display_name:None,hidden:false,hidden_triangles:Vec::new() },
            ],
        };
        // This was 35 (MAP copy) for BOTH source classes before the fix.
        let copy = world.next_copy_key(true, 35).unwrap();
        assert_eq!(copy, NPC_KEY_BASE + 8);
        assert_eq!(world.next_copy_key(false, 35).unwrap(), 35);
        world.draws.push(DrawInfo { id: 2,terrain:false,index:copy,
            center:[0.0;3],radius:1.0,original_center:[0.0;3],
            offset:[0.0;3], rotation:[0.0;3],scale:[1.0;3],
            vertex_start:0,vertex_end:0,source_ref:Some((false,NPC_KEY_BASE + 7)),
            editor_uid:Some(3),display_name:None,hidden:false,hidden_triangles:Vec::new() });
        assert_eq!(world.next_copy_key(true,35).unwrap(), NPC_KEY_BASE + 9);
        assert_eq!(world.next_copy_key(false,35).unwrap(),35);
    }
    #[test]
    fn duplicate_makes_independent_draw_without_changing_native_source() {
        let mut mesh = empty_model();
        mesh.positions = vec![[0.0,0.0,0.0],[1.0,0.0,0.0],[0.0,0.0,1.0]];
        mesh.rest = mesh.positions.clone();
        mesh.object_ids = vec![0;3];
        mesh.uvs = vec![[0,0];3];
        mesh.cba_tsb = vec![[0,0];3];
        mesh.modulation = vec![[128;3];3];
        mesh.fill = vec![[180;3];3];
        mesh.textured = vec![0;3];
        mesh.indices = vec![0,1,2];
        let mut w = WorldModel {
            model: Some(mesh), instances: 1, unavailable: 0, unique_models: 1,
            ground_triangles: 0, ground_vram_covered: 0, ground_textured: 0,
            npcs:Vec::new(),npc_issue:None,foreign:Vec::new(),naming:Default::default(),collision:Default::default(),foreign_vram:HashMap::new(),
            draws: vec![DrawInfo { id:0, terrain:false, index:0,
                center:[0.5,0.0,0.5], radius:1.0,
                original_center:[0.5,0.0,0.5], offset:[0.0;3],rotation:[0.0;3],scale:[1.0;3],
                vertex_start:0, vertex_end:3, source_ref:None, editor_uid:None,
                display_name:None, hidden:false, hidden_triangles:Vec::new() }],
        };
        let key = w.duplicate_native((false,0),1,10,[100.0,0.0,0.0]).unwrap();
        assert_eq!(key,(false,1));
        assert_eq!(w.draws.len(),2);
        assert_eq!(w.model.as_ref().unwrap().indices,vec![0,1,2,3,4,5]);
        assert_eq!(w.model.as_ref().unwrap().positions[0],[0.0;3]);
        assert_eq!(w.model.as_ref().unwrap().positions[3],[100.0,0.0,0.0]);
        assert!(w.set_offset(false,1,[250.0,0.0,0.0]));
        assert_eq!(w.model.as_ref().unwrap().positions[0],[0.0;3]);
        assert_eq!(w.model.as_ref().unwrap().positions[3],[250.0,0.0,0.0]);
        w.clear_created();
        assert_eq!(w.draws.len(),1);
        assert_eq!(w.model.as_ref().unwrap().indices,vec![0,1,2]);
    }
    #[test]
    fn hide_restore_and_delete_created_leave_native_mesh_and_ids_intact() {
        let mut mesh=empty_model();
        mesh.positions=vec![[0.0,0.0,0.0],[1.0,0.0,0.0],[0.0,0.0,1.0]];
        mesh.rest=mesh.positions.clone();
        mesh.object_ids=vec![0;3];
        mesh.uvs=vec![[0,0];3]; mesh.cba_tsb=vec![[0,0];3];
        mesh.modulation=vec![[128;3];3]; mesh.fill=vec![[180;3];3];
        mesh.textured=vec![0;3]; mesh.indices=vec![0,1,2];
        let mut world=WorldModel {
            model:Some(mesh),instances:1,unavailable:0,unique_models:1,
            ground_triangles:0,ground_vram_covered:0,ground_textured:0,
            npcs:Vec::new(),npc_issue:None,foreign:Vec::new(),naming:Default::default(),collision:Default::default(),foreign_vram:HashMap::new(),
            draws:vec![DrawInfo {id:0,terrain:false,index:0,
                center:[0.0;3],original_center:[0.0;3],radius:1.0,
                offset:[0.0;3],rotation:[0.0;3],scale:[1.0;3],vertex_start:0,vertex_end:3,
                source_ref:None,editor_uid:None,display_name:None,
                hidden:false,hidden_triangles:Vec::new()}],
        };
        world.set_display_name((false,0),Some("Town gate".into()));
        assert!(world.set_hidden((false,0),true));
        assert_eq!(world.model.as_ref().unwrap().indices,vec![0,0,0]);
        assert!(world.by_id(0).is_none());
        assert!(world.set_hidden((false,0),false));
        assert_eq!(world.model.as_ref().unwrap().indices,vec![0,1,2]);
        let a=world.duplicate_native((false,0),1,12,[50.0,0.0,0.0]).unwrap();
        let b=world.duplicate_native((false,0),1,13,[100.0,0.0,0.0]).unwrap();
        world.set_display_name(b,Some("Extra gate".into()));
        world.delete_created(a).unwrap();
        assert_eq!(world.draws.len(),2);
        assert_eq!(world.model.as_ref().unwrap().indices,vec![0,1,2,3,4,5]);
        assert_eq!(world.model.as_ref().unwrap().object_ids,vec![0,0,0,1,1,1]);
        assert_eq!(world.at(b.0,b.1).unwrap().display_name.as_deref(),Some("Extra gate"));
        assert_eq!(world.at(false,0).unwrap().display_name.as_deref(),Some("Town gate"));
        assert_eq!(world.model.as_ref().unwrap().positions[3],[100.0,0.0,0.0]);
        assert!(world.delete_created((false,0)).is_err());
    }

    #[test]
    fn rotation_keeps_native_rest_and_others_intact_and_survives_moving() {
        let mut mesh=empty_model();
        mesh.positions=vec![[1.0,0.0,0.0],[0.0,0.0,1.0],[9.0,4.0,2.0]];
        mesh.rest=mesh.positions.clone();
        let mut w=WorldModel {
            model: Some(mesh), instances:1, unavailable:0, unique_models:1,
            ground_triangles:0, ground_vram_covered:0, ground_textured:0,
            npcs:Vec::new(),npc_issue:None,foreign:Vec::new(),naming:Default::default(),collision:Default::default(),foreign_vram:HashMap::new(),
            draws:vec![DrawInfo {id:0,terrain:false,index:0,
                center:[0.0;3],original_center:[0.0;3],radius:1.0,
                offset:[0.0;3],rotation:[0.0;3],scale:[1.0;3],vertex_start:0,vertex_end:2,
                source_ref:None,editor_uid:None,display_name:None,hidden:false,
                hidden_triangles:Vec::new()}],
        };
        let right_angle=std::f32::consts::FRAC_PI_2;
        assert!(w.set_rotation(false,0,[0.0,right_angle,0.0]));
        let m=w.model.as_ref().unwrap();
        assert!((m.positions[0][2]+1.0).abs()<0.00001);
        assert!((m.positions[1][0]-1.0).abs()<0.00001);
        assert_eq!(m.positions[2],[9.0,4.0,2.0]);
        assert_eq!(m.rest[0],[1.0,0.0,0.0]);
        assert!(w.set_offset(false,0,[10.0,0.0,0.0]));
        assert!((w.model.as_ref().unwrap().positions[1][0]-11.0).abs()<0.00001);
        assert!(w.set_rotation(false,0,[0.0;3]));
        assert_eq!(w.model.as_ref().unwrap().positions[0],[11.0,0.0,0.0]);
        assert!(w.set_offset(false,0,[0.0;3]));
        assert_eq!(w.model.as_ref().unwrap().positions,w.model.as_ref().unwrap().rest);
    }
    #[test]
    fn scale_is_reversible_and_preserves_rest_and_unselected_geometry() {
        let mut mesh=empty_model();
        mesh.positions=vec![[2.0,0.0,0.0],[0.0,0.0,3.0],[99.0,8.0,0.0]];
        mesh.rest=mesh.positions.clone();
        let mut w=WorldModel { model:Some(mesh),instances:1,unavailable:0,unique_models:1,
            ground_triangles:0,ground_vram_covered:0,ground_textured:0,
            npcs:Vec::new(),npc_issue:None,foreign:Vec::new(),naming:Default::default(),collision:Default::default(),foreign_vram:HashMap::new(),
            draws:vec![DrawInfo{id:0,terrain:false,index:0,center:[0.0;3],
                original_center:[0.0;3],radius:3.0,offset:[0.0;3],rotation:[0.0;3],scale:[1.0;3],
                vertex_start:0,vertex_end:2,source_ref:None,editor_uid:None,
                display_name:None,hidden:false,hidden_triangles:Vec::new()}] };
        assert!(w.set_scale(false,0,[2.0,1.0,0.5]));
        assert_eq!(w.model.as_ref().unwrap().positions[0],[4.0,0.0,0.0]);
        assert_eq!(w.model.as_ref().unwrap().positions[1],[0.0,0.0,1.5]);
        assert_eq!(w.model.as_ref().unwrap().positions[2],[99.0,8.0,0.0]);
        assert!(!w.set_scale(false,0,[f32::NAN,1.0,1.0]));
        assert!(!w.set_scale(false,0,[0.0,1.0,1.0]));
        assert_eq!(w.at(false,0).unwrap().scale,[2.0,1.0,0.5]);
        assert!(w.set_rotation(false,0,[0.0,std::f32::consts::FRAC_PI_2,0.0]));
        assert!(w.set_offset(false,0,[5.0,0.0,0.0]));
        assert!(w.set_scale(false,0,[1.0;3]));
        assert!(w.set_rotation(false,0,[0.0;3]));
        assert!(w.set_offset(false,0,[0.0;3]));
        assert_eq!(w.model.as_ref().unwrap().positions,w.model.as_ref().unwrap().rest);
    }
    #[test]
    fn rotation_rejects_invalid_values_without_editing_mesh() {
        let mut mesh=empty_model(); mesh.positions=vec![[0.0;3]]; mesh.rest=mesh.positions.clone();
        let mut w=WorldModel {model:Some(mesh),instances:1,unavailable:0,unique_models:1,
            ground_triangles:0,ground_vram_covered:0,ground_textured:0,
            npcs:Vec::new(),npc_issue:None,foreign:Vec::new(),naming:Default::default(),collision:Default::default(),foreign_vram:HashMap::new(),
            draws:vec![DrawInfo{id:0,terrain:false,index:0,center:[0.0;3],
                original_center:[0.0;3],radius:1.0,offset:[0.0;3],rotation:[0.0;3],scale:[1.0;3],
                vertex_start:0,vertex_end:1,source_ref:None,editor_uid:None,
                display_name:None,hidden:false,hidden_triangles:Vec::new()}]};
        assert!(!w.set_rotation(false,0,[f32::NAN,0.0,0.0]));
        assert!(!w.set_rotation(false,0,[0.0,1000.1,0.0]));
        assert_eq!(w.model.as_ref().unwrap().positions,vec![[0.0;3]]);
        assert_eq!(w.at(false,0).unwrap().rotation,[0.0;3]);
    }
    #[test]
    fn a_piece_can_move_and_return_without_touching_other_vertices() {
        let mut m = empty_model();
        m.positions = vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]];
        m.rest = m.positions.clone();
        let mut w = WorldModel {
            model: Some(m), instances: 1, unavailable: 0, unique_models: 1,
            ground_triangles: 0, ground_vram_covered: 0, ground_textured: 0,
            npcs:Vec::new(),npc_issue:None,foreign:Vec::new(),naming:Default::default(),collision:Default::default(),foreign_vram:HashMap::new(),
            draws: vec![DrawInfo {id:0, terrain:false, index:0,
                center:[1.0,2.0,3.0],radius:1.0,
                original_center:[1.0,2.0,3.0],offset:[0.0;3],rotation:[0.0;3],scale:[1.0;3],
                vertex_start:0,vertex_end:1,source_ref:None,editor_uid:None,
                display_name:None, hidden:false, hidden_triangles:Vec::new() }],
        };
        assert!(w.set_offset(false, 0, [32.0,-8.0,64.0]));
        assert_eq!(w.model.as_ref().unwrap().positions[0], [33.0,-6.0,67.0]);
        assert_eq!(w.model.as_ref().unwrap().positions[1], [4.0,5.0,6.0]);
        assert!(w.set_offset(false, 0, [0.0;3]));
        assert_eq!(w.model.as_ref().unwrap().positions, w.model.as_ref().unwrap().rest);
    }
}

impl WorldModel {
    pub fn npc(&self, key: (bool, usize)) -> Option<&NpcInfo> {
        if key.0 || key.1 < NPC_KEY_BASE { return None; }
        self.npcs.iter().find(|npc| npc.key == key.1)
    }

    pub fn by_id(&self, id: u32) -> Option<&DrawInfo> {
        self.draws.get(id as usize).filter(|d| d.id == id && !d.hidden)
    }
    pub fn at(&self, terrain: bool, index: usize) -> Option<&DrawInfo> {
        self.draws.iter().find(|d| d.terrain == terrain && d.index == index)
    }
}


/// Accurate original PS1 geometry/rotation from a donor scene, transformed into
/// LCK's viewer-coordinate system. Actor MAN script duplication is not covered.
pub(super) fn foreign_original_geometry(donor:&AssembledScene,kind:&str,record:usize)
    ->Result<(Model,Vec<[f32;3]>),String> {
    let draw=match kind {
        "placement"=>Some(donor.placements.get(record).ok_or("Original MAP object missing")?),
        "terrain"=>Some(donor.terrain.get(record).ok_or("Original terrain object missing")?),
        "mesh"=>None,
        _=>return Err("Only original MAP/terrain/TMD assets can be imported as geometry; MAN actors still need animation/script and model relocation".into()),
    };
    let slot=if let Some(d)=draw {d.res_tmd}
        else {*donor.env_tmds.get(record).ok_or("Original TMD slot missing")?};
    let raw=donor.res.tmds.get(slot).ok_or("Original donor TMD missing")?;
    let mesh=Model::from_tmd(&raw.raw).ok_or("Original donor TMD could not be decoded")?;
    let points=if let Some(d)=draw {
        let native=donor.draw_world_position(d);
        let lift=[native[0]-d.world_x as f32,native[1]-d.world_y as f32,
            native[2]-d.world_z as f32];
        mesh.positions.iter().map(|&p|{
            let p=d.place_point(p,lift); [p[0],-p[1],p[2]]
        }).collect()
    } else {
        mesh.positions.iter().map(|p|[p[0],-p[1],p[2]]).collect()
    };
    Ok((mesh,points))
}

pub(super) fn foreign_size(donor:&AssembledScene,kind:&str,record:usize)->Result<(f32,f32),String>{
    let (_,points)=foreign_original_geometry(donor,kind,record)?;
    if points.is_empty(){return Err("Foreign model has no vertices".into());}
    let (center,radius)=bounds(&points);
    let bottom=points.iter().map(|p|p[1]).fold(f32::INFINITY,f32::min);
    Ok((radius,bottom-center[1]))
}

fn empty_model() -> Model {
    Model {
        positions: Vec::new(), rest: Vec::new(), object_ids: Vec::new(),
        uvs: Vec::new(), cba_tsb: Vec::new(), modulation: Vec::new(),
        fill: Vec::new(), textured: Vec::new(), indices: Vec::new(),
        center: [0.0; 3], radius: 1.0,
    }
}

fn bounds(positions: &[[f32; 3]]) -> ([f32; 3], f32) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for p in positions {
        for k in 0..3 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); }
    }
    let center = [0, 1, 2].map(|k| (lo[k] + hi[k]) * 0.5);
    let radius = positions.iter().map(|p| {
        let d = [0, 1, 2].map(|k| p[k] - center[k]);
        (d[0]*d[0] + d[1]*d[1] + d[2]*d[2]).sqrt()
    }).fold(1.0_f32, f32::max);
    (center, radius)
}


/// Copy the original game's textured ground quads into the CPU render stream.
///
/// Each walk-ground vertex already has an atlas UV, PS1 [CLUT, tpage], and
/// GP0 modulation colour. The old editor overwrote all of them with zero,
/// then forced `textured = 0`, so the rasterizer could only show grey.
/// The general-purpose rasterizer already knows how to sample scene VRAM.
///
/// If older/malformed scene data lacks the complete native texture attributes,
/// retain the previous diagnostic-grey fallback without misaligning streams.
fn append_native_ground(
    out: &mut Model,
    hf: &legaia_scene_formats::field_objects::WalkHeightfield,
) -> usize {
    let vertices = legaia_engine_scene::field_ground::render_positions(hf);
    let indices = legaia_engine_scene::field_ground::render_indices(hf);
    if vertices.is_empty() || indices.is_empty() { return 0; }
    let Ok(base) = u32::try_from(out.positions.len()) else { return 0; };
    let Ok(count) = u32::try_from(vertices.len()) else { return 0; };
    if base.checked_add(count).is_none() { return 0; }

    let n = vertices.len();
    // The builder emits one UV/page/palette/colour per position. Check all
    // vectors together to preserve the renderer's attribute indexing.
    let textured = hf.uvs.len() == n && hf.cba_tsb.len() == n && hf.colors.len() == n;
    out.positions.extend(vertices.iter().map(|p| [p[0], -p[1], p[2]]));
    // Synthetic floor is visible but cannot be selected as a world object.
    out.object_ids.extend(std::iter::repeat_n(u32::MAX, n));
    if textured {
        out.uvs.extend_from_slice(&hf.uvs);
        out.cba_tsb.extend_from_slice(&hf.cba_tsb);
        out.modulation.extend_from_slice(&hf.colors);
        out.textured.extend(std::iter::repeat_n(1, n));
    } else {
        out.uvs.extend(std::iter::repeat_n([0, 0], n));
        out.cba_tsb.extend(std::iter::repeat_n([0, 0], n));
        out.modulation.extend(std::iter::repeat_n([128, 128, 128], n));
        out.textured.extend(std::iter::repeat_n(0, n));
    }
    // Used only in untextured diagnostic fallback / viewer-light mode.
    out.fill.extend(std::iter::repeat_n([94, 103, 96], n));
    out.indices.extend(indices.iter().map(|&i| base + i));
    indices.len() / 3
}

/// Native TMD instances plus native walk-ground collision heightfield.
pub(super) fn build_world(
    scene: &AssembledScene,
    index: &legaia_engine_scene::scene::ProtIndex,
) -> WorldModel {
    let mut cache: HashMap<usize, Option<Model>> = HashMap::new();
    let mut out = empty_model();
    let mut instances = 0usize;
    let mut unavailable = 0usize;
    let mut draws = Vec::new();
    let mut ground_triangles = 0usize;

    if let Some(hf) = &scene.ground {
        ground_triangles = append_native_ground(&mut out, hf);
    }
    // The scene's existing VRAM contains every original TIM upload we know.
    // Audit the ground while it is the only mesh, once per scene assembly.
    // This distinguishes missing ground UVs from missing native atlas pages.
    let (ground_vram_covered, ground_textured) = out.texture_coverage(&scene.res.vram);

    // IDs match this draw order and are carried in Model::object_ids, allowing
    // object-level selection and highlighting without new geometry formats.
    for (terrain, index, draw) in scene.terrain.iter().enumerate().map(|(i,d)| (true,i,d))
        .chain(scene.placements.iter().enumerate().map(|(i,d)| (false,i,d))) {
        let mesh = cache.entry(draw.res_tmd).or_insert_with(|| {
            scene.res.tmds.get(draw.res_tmd)
                .and_then(|tmd| Model::from_tmd(&tmd.raw))
        });
        let Some(mesh) = mesh.as_ref() else { unavailable += 1; continue; };
        let Ok(base) = u32::try_from(out.positions.len()) else { unavailable += 1; continue; };
        let Ok(count) = u32::try_from(mesh.positions.len()) else { unavailable += 1; continue; };
        if base.checked_add(count).is_none() { unavailable += 1; continue; }
        let Ok(id) = u32::try_from(draws.len()) else { unavailable += 1; continue; };
        let native_position = scene.draw_world_position(draw);
        let lift = [
            native_position[0] - draw.world_x as f32,
            native_position[1] - draw.world_y as f32,
            native_position[2] - draw.world_z as f32,
        ];
        let positions: Vec<[f32; 3]> = mesh.positions.iter().map(|&p| {
            let p = draw.place_point(p, lift);
            [p[0], -p[1], p[2]]
        }).collect();
        let (center, radius) = bounds(&positions);
        out.positions.extend_from_slice(&positions);
        out.object_ids.extend(std::iter::repeat_n(id, positions.len()));
        out.indices.extend(mesh.indices.iter().map(|i| base + *i));
        out.uvs.extend_from_slice(&mesh.uvs);
        out.cba_tsb.extend_from_slice(&mesh.cba_tsb);
        out.modulation.extend_from_slice(&mesh.modulation);
        out.fill.extend_from_slice(&mesh.fill);
        out.textured.extend_from_slice(&mesh.textured);
        draws.push(DrawInfo {
            id, terrain, index, center, radius, original_center: center,
            offset: [0.0; 3], rotation: [0.0;3], scale: [1.0;3], vertex_start: base as usize,
            vertex_end: base as usize + positions.len(),
            source_ref: None, editor_uid: None,
            display_name: None, hidden: false, hidden_triangles: Vec::new(),
        });
        instances += 1;
    }
    // Retain metadata rows for native records whose TMD could not be decoded.
    // They can still be named and hidden in the hierarchy; never synthesize geometry.
    // Append after real draws so rasterizer object IDs remain unchanged.
    let present: std::collections::HashSet<_> = draws.iter()
        .map(|d: &DrawInfo| (d.terrain,d.index)).collect();
    for (terrain,index,d) in scene.terrain.iter().enumerate().map(|(i,d)|(true,i,d))
        .chain(scene.placements.iter().enumerate().map(|(i,d)|(false,i,d))) {
        if present.contains(&(terrain,index)) { continue; }
        let Ok(id)=u32::try_from(draws.len()) else {break};
        let center=[d.world_x as f32,-(d.world_y as f32),d.world_z as f32];
        draws.push(DrawInfo {
            id, terrain, index, center, original_center:center, radius:0.0,
            offset:[0.0;3],rotation:[0.0;3],scale:[1.0;3],vertex_start:0,vertex_end:0,
            source_ref:None,editor_uid:None,display_name:None,
            hidden:false,hidden_triangles:Vec::new(),
        });
    }
    // Native MAN actor records: drawn in the same ID/VRAM stream as terrain,
    // but retain distinct native keys so selection cannot collide with MAP.
    let (npcs, npc_issue) = npcs::append_scene_npcs(scene, index, &mut out, &mut draws);
    let model = if out.indices.is_empty() { None } else {
        out.rest = out.positions.clone();
        out.refit();
        Some(out)
    };
    WorldModel {
        naming:super::naming::inventory(scene,index,&npcs),
        model, instances, unavailable,
        unique_models: cache.values().filter(|v| v.is_some()).count(),
        ground_triangles, ground_vram_covered, ground_textured, draws,
        npcs, npc_issue, foreign:Vec::new(),
        collision:super::collision_recipes::source_inventory(scene,index).unwrap_or_else(|error| {
            log::debug!("Collision provenance unavailable: {error}");Default::default()
        }),foreign_vram:HashMap::new(),
    }
}

#[cfg(test)]
mod ground_texture_tests {
    use super::*;
    use legaia_scene_formats::field_objects::WalkHeightfield;

    fn one_tile() -> WalkHeightfield {
        WalkHeightfield {
            positions: vec![[0.0, 0.0, 0.0], [128.0, 0.0, 0.0],
                            [0.0, 0.0, 128.0], [128.0, 0.0, 128.0]],
            uvs: vec![[32, 63], [63, 63], [32, 32], [63, 32]],
            cba_tsb: vec![[0x7C40, 0x001A]; 4],
            colors: vec![[128, 128, 128]; 4],
            indices: vec![0, 1, 2, 1, 3, 2],
            ..WalkHeightfield::default()
        }
    }

    #[test]
    fn ground_uses_native_uv_clut_page_and_modulation() {
        let hf = one_tile();
        let mut m = empty_model();
        assert_eq!(append_native_ground(&mut m, &hf), 2);
        assert_eq!(m.uvs, hf.uvs);
        assert_eq!(m.cba_tsb, hf.cba_tsb);
        assert_eq!(m.modulation, hf.colors);
        assert_eq!(m.textured, vec![1; 4]);
        assert_eq!(m.object_ids, vec![u32::MAX; 4]);
        assert_eq!(m.indices.len(), 6);
    }

    #[test]
    fn incomplete_native_attributes_fall_back_to_diagnostic_fill() {
        let mut hf = one_tile();
        hf.cba_tsb.clear();
        let mut m = empty_model();
        assert_eq!(append_native_ground(&mut m, &hf), 2);
        assert_eq!(m.textured, vec![0; 4]);
        assert_eq!(m.cba_tsb, vec![[0, 0]; 4]);
        assert_eq!(m.fill, vec![[94, 103, 96]; 4]);
        assert_eq!(m.indices.len(), 6);
    }
}
