//! Properties-only native wall-grid authoring. Spatial display never assigns
//! native ownership; only an explicit gesture changes the selected draft.
use super::{collision_recipes::{parse_cells, SourceProvenance},world_view::WorldModel};
use eframe::egui::{self,Color32,Pos2,Rect,Vec2};
use std::{collections::BTreeSet,sync::Arc};

#[derive(Clone,Copy,Default,PartialEq)]
enum Tool {#[default] Block, Open, Unspecified}
#[derive(Clone)]
struct GridDraft {
    origin:[i32;2],destination:bool,tool:Tool,
    rectangle:[i32;4],anchor:Option<[i32;2]>,error:Option<String>,
}
impl GridDraft {
    fn new(source:&SourceProvenance)->Self {
        let cell=source.object.cell;
        Self{origin:[(cell[0] as i32*2-8).clamp(0,240),(cell[1] as i32*2-6).clamp(0,240)],
            destination:false,tool:Tool::Block,rectangle:[0;4],anchor:None,error:None}
    }
}

fn translation(offset:[f32;3])->Result<[i32;2],String> {
    if offset[1]!=0.0 || offset.iter().any(|v|!v.is_finite()||v.abs()>32768.0) || offset[0]%64.0!=0.0 || offset[2]%64.0!=0.0 {
        return Err("Wall authoring requires exact 64-unit X/Z translation and unchanged Y".into());
    }
    Ok([(offset[0]/64.0) as i32,(offset[2]/64.0) as i32])
}

fn edit_region(walls:&mut String,open:&mut String,a:[i32;2],b:[i32;2],tool:Tool)->Result<(),String> {
    if a.iter().chain(&b).any(|v|!(0..256).contains(v)) {return Err("Region exceeds native sub-cell grid 0..255".into());}
    let mut blocking:BTreeSet<_>=parse_cells(walls)?.into_iter().collect();
    let mut nonblocking:BTreeSet<_>=parse_cells(open)?.into_iter().collect();
    for z in a[1].min(b[1])..=a[1].max(b[1]) {for x in a[0].min(b[0])..=a[0].max(b[0]) {
        let cell=[x,z];blocking.remove(&cell);nonblocking.remove(&cell);
        match tool {Tool::Block=>{blocking.insert(cell);},Tool::Open=>{nonblocking.insert(cell);},Tool::Unspecified=>{}}
    }}
    let text=|cells:BTreeSet<[i32;2]>|cells.into_iter().map(|[x,z]|format!("{x},{z}")).collect::<Vec<_>>().join("\n");
    *walls=text(blocking);*open=text(nonblocking);Ok(())
}

fn wall(map:&[u8],cell:[i32;2])->bool {
    if cell.iter().any(|v|!(0..256).contains(v)) {return false;}
    let [x,z]=cell;let offset=0x4000+(z as usize/2)*128+x as usize/2;
    map[offset]&(1<<(4+z%2*2+x%2))!=0
}

fn pointer_cell(rect:Rect,pos:Pos2,origin:[i32;2])->Option<[i32;2]> {
    if !rect.contains(pos) {return None;}
    Some([origin[0]+(((pos.x-rect.left())/rect.width()*16.0) as i32).min(15),
        origin[1]+(((pos.y-rect.top())/rect.height()*16.0) as i32).min(15)])
}

fn generated_cells(ui:&egui::Ui,g:&super::collision_generation::GeneratedCollision,offset:[f32;3],rotation:[f32;3],scale:[f32;3])
    ->Result<Arc<BTreeSet<[i32;2]>>,String> {
    let cache=egui::Id::new(("generated_wall_grid",&g.mesh_sha256,g.band_height.to_bits(),g.min_face_area.to_bits(),
        offset.map(f32::to_bits),rotation.map(f32::to_bits),scale.map(f32::to_bits)));
    ui.ctx().data_mut(|d|d.get_temp::<Result<Arc<BTreeSet<[i32;2]>>,String>>(cache)).unwrap_or_else(||{
        let result=g.wall_cells(offset,rotation,scale).map(Arc::new);
        ui.ctx().data_mut(|d|d.insert_temp(cache,result.clone()));result
    })
}

pub(super) fn show(ui:&mut egui::Ui,world:&WorldModel,key:(bool,usize),source:&SourceProvenance,
    index:&legaia_engine_scene::scene::ProtIndex,walls:&mut String,open:&mut String) {
    let id=ui.id().with(("native_collision_grid",format!("{:?}",world.collision.owner(world,key)),&source.map_sha256));
    let mut state=ui.ctx().data_mut(|d|d.get_temp::<GridDraft>(id)).unwrap_or_else(||GridDraft::new(source));
    // Read/verify once per immutable MAP fingerprint, not once per frame.
    let cache_id=egui::Id::new(("native_collision_grid_map",source.object.map_entry,&source.map_sha256));
    let cached=ui.ctx().data_mut(|d|d.get_temp::<Result<Arc<Vec<u8>>,String>>(cache_id));
    let map=cached.unwrap_or_else(||{
        let result=index.entry_bytes(source.object.map_entry).map_err(|e|e.to_string()).and_then(|bytes|{
            if bytes.len()!=0x12000 || super::collision_recipes::digest(&bytes)!=source.map_sha256 {
                Err("Native MAP fingerprint changed; reload the scene".into())
            } else {Ok(bytes)}
        });
        ui.ctx().data_mut(|d|d.insert_temp(cache_id,result.clone()));result
    });
    let map=match map {Ok(map)=>map,Err(error)=>{ui.colored_label(Color32::LIGHT_RED,error);return;}};
    let Some(draw)=world.at(key.0,key.1) else {return;};
    let delta=translation(draw.offset);
    let supported=delta.is_ok() && draw.rotation==[0.0;3] && draw.scale==[1.0;3];
    ui.label(format!("MAP {} • descriptor {} • source cell {:?}",source.object.map_entry,source.object.descriptor_id,source.object.cell));
    ui.small("16 × 16 native wall sub-cells, 64 PS1 units each. Drag a rectangle or enter exact bounds. Save authored recipe applies the draft through undo/redo.");
    ui.small("Painting is optional. Unspecified removes a manual correction and reveals the automatic default. Nonblocking cannot erase native or other-instance walls.");
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut state.tool,Tool::Block,"Blocking");
        ui.selectable_value(&mut state.tool,Tool::Open,"Nonblocking");
        ui.selectable_value(&mut state.tool,Tool::Unspecified,"Unspecified");
    });
    ui.small("Amber = original wall (owner unknown); cyan = selected draft block; green = selected open requirement; purple = other saved contributor; red = open-space conflict. Native observations stay protected.");
    ui.horizontal_wrapped(|ui| {
        let changed=ui.checkbox(&mut state.destination,"View destination grid").changed();
        if changed {if let Ok(d)=delta.as_ref() {
            let sign=if state.destination {1}else{-1};
            for k in 0..2 {state.origin[k]=(state.origin[k]+sign*d[k]).clamp(0,240);}
        }}
        ui.label("Origin X,Z");
        for v in &mut state.origin {ui.add(egui::DragValue::new(v).range(0..=240));}
        if ui.button("Center source").clicked() {state.origin=GridDraft::new(source).origin;state.destination=false;}
    });
    let shift=if state.destination {delta.as_ref().copied().unwrap_or([0;2])} else {[0;2]};
    let blocking=parse_cells(walls).map(|c|c.into_iter().collect::<BTreeSet<_>>());
    let nonblocking=parse_cells(open).map(|c|c.into_iter().collect::<BTreeSet<_>>());
    let generated=world.collision.recipes.iter().find(|a|Some(&a.owner)==world.collision.owner(world,key).as_ref())
        .and_then(|a|a.recipe.generated.as_ref());
    let selected_generated=generated.map(|g|{
        let offset=if state.destination {draw.offset}else{[0.0;3]};
        let rotation=if state.destination {draw.rotation}else{[0.0;3]};
        let scale=if state.destination {draw.scale}else{[1.0;3]};
        generated_cells(ui,g,offset,rotation,scale)
    });
    if let Some(Err(error))=&selected_generated {ui.colored_label(Color32::LIGHT_RED,format!("Generated collision: {error}"));}
    let mut others=BTreeSet::new();
    let owner=world.collision.owner(world,key);
    for assignment in &world.collision.recipes {
        if Some(&assignment.owner)==owner.as_ref() {continue;}
        let Some(other)=world.draws.iter().find(|d|world.collision.owner(world,(d.terrain,d.index)).as_ref()==Some(&assignment.owner)) else {continue;};
        if other.hidden {continue;}
        let d=translation(other.offset);
        if let Some(g)=&assignment.recipe.generated {
            if let Ok(cells)=generated_cells(ui,g,other.offset,other.rotation,other.scale) {
                for &cell in cells.iter() {
                    let excluded=d.as_ref().is_ok_and(|d|assignment.recipe.nonblocking.contains(&[cell[0]-d[0],cell[1]-d[1]]));
                    if !excluded {others.insert(cell);}
                }
            }
        }
        if other.rotation==[0.0;3] && other.scale==[1.0;3] {
            if let Ok(d)=d {for c in &assignment.recipe.walls {others.insert([c[0]+d[0],c[1]+d[1]]);}}
        }
    }
    let side=ui.available_width().min(448.0).max(160.0);
    let (rect,response)=ui.allocate_exact_size(Vec2::splat(side),egui::Sense::click_and_drag());
    let painter=ui.painter_at(rect);let size=side/16.0;
    for row in 0..16 {for col in 0..16 {
        let cell=[state.origin[0]+col,state.origin[1]+row];let local=[cell[0]-shift[0],cell[1]-shift[1]];
        let native=wall(&map,cell);let other=state.destination && others.contains(&cell);
        let free=nonblocking.as_ref().is_ok_and(|c|c.contains(&local));
        let block=blocking.as_ref().is_ok_and(|c|c.contains(&local)) || (!free && selected_generated.as_ref().is_some_and(|c|c.as_ref().is_ok_and(|c|c.contains(&cell))));
        let color=if free&&(native||other||block) {Color32::from_rgb(180,45,55)} else if block {Color32::from_rgb(20,130,170)}
            else if free {Color32::from_rgb(25,100,65)} else if native {Color32::from_rgb(140,100,35)}
            else if other {Color32::from_rgb(100,65,140)} else {Color32::from_gray(30)};
        let cell_rect=Rect::from_min_size(rect.min+Vec2::new(col as f32*size,row as f32*size),Vec2::splat(size));
        painter.rect_filled(cell_rect.shrink(0.5),0.0,color);
        if native {painter.circle_filled(cell_rect.center(),2.0,Color32::YELLOW);}
    }}
    for i in 0..=16 {
        let stroke=egui::Stroke::new(if (state.origin[0]+i)%2==0 {1.0_f32}else{0.5_f32},Color32::GRAY);
        painter.line_segment([rect.min+Vec2::new(i as f32*size,0.0),rect.min+Vec2::new(i as f32*size,side)],stroke);
        let stroke=egui::Stroke::new(if (state.origin[1]+i)%2==0 {1.0_f32}else{0.5_f32},Color32::GRAY);
        painter.line_segment([rect.min+Vec2::new(0.0,i as f32*size),rect.min+Vec2::new(side,i as f32*size)],stroke);
    }
    let hovered=ui.input(|i|i.pointer.hover_pos()).and_then(|p|pointer_cell(rect,p,state.origin));
    if let Some(cell)=hovered {
        let offset=0x4000+(cell[1] as usize/2)*128+cell[0] as usize/2;
        let bit=4+cell[1]%2*2+cell[0]%2;
        ui.monospace(format!("X,Z {},{} • MAP {offset:#x}, bit {bit} • world center ({},{}) • floor tier {}",
            cell[0],cell[1],cell[0]*64+32,(cell[1]-2)*64+32,map[offset]&15));
        ui.small("World center uses native wall-query bias. Floor tier alone does not establish walkable height.");
        if response.hovered() && ui.input(|i|i.pointer.primary_pressed()) {state.anchor=Some(cell);}
    }
    let mut region=None;
    if let (Some(start),Some(end))=(state.anchor,hovered) {
        let min=[start[0].min(end[0]),start[1].min(end[1])];let max=[start[0].max(end[0]),start[1].max(end[1])];
        let selection=Rect::from_min_max(rect.min+Vec2::new((min[0]-state.origin[0]) as f32*size,(min[1]-state.origin[1]) as f32*size),
            rect.min+Vec2::new((max[0]-state.origin[0]+1) as f32*size,(max[1]-state.origin[1]+1) as f32*size));
        painter.rect_stroke(selection,0.0,egui::Stroke::new(2.0_f32,Color32::WHITE),egui::StrokeKind::Inside);
        if ui.input(|i|i.pointer.primary_released()) {region=Some((start,end));}
    }
    if ui.input(|i|i.pointer.primary_released()) {state.anchor=None;}
    ui.horizontal_wrapped(|ui| {
        ui.label("Region X0,Z0,X1,Z1 (inclusive)");
        for v in &mut state.rectangle {ui.add(egui::DragValue::new(v).range(0..=255));}
        if ui.add_enabled(supported,egui::Button::new("Paint region")).clicked() {
            let [x,z,x1,z1]=state.rectangle;region=Some(([x,z],[x1,z1]));
        }
    });
    if let Some((a,b))=region {
        state.error=if !supported {Some("Rotation, scale, Y movement or unquantized translation prevents wall authoring".into())}
            else {edit_region(walls,open,[a[0]-shift[0],a[1]-shift[1]],[b[0]-shift[0],b[1]-shift[1]],state.tool).err()};
    }
    if !supported {ui.colored_label(Color32::LIGHT_RED,"Unsupported transform: walls require exact X/Z translation, unchanged Y, rotation and scale.");}
    if let Err(error)=blocking.as_ref() {ui.colored_label(Color32::LIGHT_RED,error);}
    if let Err(error)=nonblocking.as_ref() {ui.colored_label(Color32::LIGHT_RED,error);}
    if let Some(error)=&state.error {ui.colored_label(Color32::LIGHT_RED,error);}
    ui.ctx().data_mut(|d|d.insert_temp(id,state));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn region_tools_are_explicit_and_transactional() {
        let mut blocks=String::new();let mut open=String::new();
        edit_region(&mut blocks,&mut open,[3,3],[2,2],Tool::Block).unwrap();
        assert_eq!(parse_cells(&blocks).unwrap(),vec![[2,2],[2,3],[3,2],[3,3]]);
        edit_region(&mut blocks,&mut open,[2,2],[2,3],Tool::Open).unwrap();
        assert_eq!(parse_cells(&blocks).unwrap(),vec![[3,2],[3,3]]);
        assert_eq!(parse_cells(&open).unwrap(),vec![[2,2],[2,3]]);
        let before=(blocks.clone(),open.clone());
        assert!(edit_region(&mut blocks,&mut open,[-1,0],[2,2],Tool::Block).is_err());
        assert_eq!((blocks.clone(),open.clone()),before);
        edit_region(&mut blocks,&mut open,[0,0],[255,255],Tool::Unspecified).unwrap();
        assert!(blocks.is_empty()&&open.is_empty());
    }
    #[test]
    fn pointer_and_translation_use_exact_native_cells() {
        let rect=Rect::from_min_size(Pos2::ZERO,Vec2::splat(320.0));
        assert_eq!(pointer_cell(rect,Pos2::new(31.0,41.0),[180,150]),Some([181,152]));
        assert_eq!(translation([64.0,0.0,-128.0]).unwrap(),[1,-2]);
        assert!(translation([1.0,0.0,0.0]).is_err());
        assert!(translation([0.0,1.0,0.0]).is_err());
    }
}
