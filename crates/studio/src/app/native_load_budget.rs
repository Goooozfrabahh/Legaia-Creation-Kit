//! Static request accounting derived from original SCUS dispatch arms. This
//! reports allocations, not free memory: actors, rendering and fragmentation
//! require an actual runtime capture. It never gates unrestricted export.
use anyhow::{Result,ensure};
use serde::Serialize;

#[derive(Serialize)]
pub(super) struct Event {
    carrier:String,kind:u8,decoded_or_raw_bytes:usize,compressed:bool,
    destination_allocation_bytes:usize,lzs_scratch_bytes:usize,
    lifetime:&'static str,persistent_requested_before:usize,
    accounted_request_peak:usize,
}
#[derive(Serialize)]
pub(super) struct Profile {
    schema:u32,evidence_class:&'static str,
    native_model_count:usize,global_model_count_including_resident_five:usize,
    fixed_asset_arena_bytes:usize,retail_heap_arena_bytes:usize,
    persistent_destination_requests:usize,peak_accounted_requests:usize,
    events:Vec<Event>,omitted_runtime_costs:Vec<&'static str>,
    live_heap_capacity_verified:bool,original_engine_instruction_simulation_verified:bool,
    native_render_verified:bool,native_gameplay_verified:bool,
    warnings:Vec<String>,
}
pub(super) fn analyze(bundle:&[u8],pac:Option<&[u8]>)->Result<Profile>{
    let(base,_)=super::native_asset_plan::scene_registration::native_descriptor_table(bundle)?;
    let trace=super::native_loader_trace::trace(bundle,base,pac)?;
    let mut persistent=0usize;let mut peak=0usize;let mut events=vec![];let mut warnings=trace.gaps.clone();
    for (carrier,kind,size) in &trace.dispatch {
        let compressed=carrier.starts_with("bundle")&&!matches!(*kind,10|15|20);
        let (destination,lifetime)=match kind{
            0=>(if compressed{0x11800}else{0},"temporary; freed after one TIM; copy-only stream uses source arena"),
            1=>(if compressed{0x70800}else{0},"temporary list staging; copy-only stream uses source arena"),
            2..=9=>(size.div_ceil(4)*4,"persistent native destination; renderer/actor allocations additional"),
            11=>(size.div_ceil(4)*4,"replacement MOVE destination; prior cleanup not accounted here"),
            _=>(0,"no destination allocation in audited dispatcher arm"),
        };
        let scratch=if compressed&&destination>0{0x1014}else{0};
        let simultaneous=persistent.checked_add(destination).and_then(|v|v.checked_add(scratch)).ok_or_else(||anyhow::anyhow!("Native request accounting overflow"))?;
        peak=peak.max(simultaneous);
        events.push(Event{carrier:carrier.clone(),kind:*kind,decoded_or_raw_bytes:*size,compressed,destination_allocation_bytes:destination,lzs_scratch_bytes:scratch,lifetime,persistent_requested_before:persistent,accounted_request_peak:simultaneous});
        if matches!(*kind,2..=9|11){persistent=persistent.checked_add(destination).ok_or_else(||anyhow::anyhow!("Persistent request overflow"))?;}
        if *kind==0&&*size>0x11800{warnings.push(format!("Single TIM declares {size} bytes; original fixed destination is 71680 bytes"));}
        if *kind==1&&compressed&&*size>0x70800{warnings.push(format!("TIM list declares {size} bytes; original fixed destination is 460800 bytes"));}
    }
    warnings.push("Live allocation success, fragmentation, GPU completion and actor/render costs are NOT verified by this accounting".into());
    if peak+0x62c00>0x134800{warnings.push("Accounted request peak plus fixed asset arena exceeds the retail heap arena even before omitted costs".into());}
    if trace.models.len()+5>super::native_asset_plan::scene_registration::FIRST_KNOWN_MODEL_CONFLICT{warnings.push("Model registrations reach a known unrelated native global array; unrestricted attempt retained".into());}
    ensure!(events.len()<=4096,"Unbounded native request profile");
    Ok(Profile{schema:1,evidence_class:"static native dispatcher request accounting; not runtime execution or available-memory proof",native_model_count:trace.models.len(),global_model_count_including_resident_five:trace.models.len()+5,fixed_asset_arena_bytes:0x62c00,retail_heap_arena_bytes:0x134800,persistent_destination_requests:persistent,peak_accounted_requests:peak,events,
        omitted_runtime_costs:vec!["resident character and effect allocations","field sidecar, actor pools and render-node allocations","GPU packet buffers, ordering tables, audio and driver allocations","loader caller allocations, cleanup order and heap fragmentation","allocation failure paths and subsequent native GPU writes"],live_heap_capacity_verified:false,original_engine_instruction_simulation_verified:false,native_render_verified:false,native_gameplay_verified:false,warnings})
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn compressed_single_tim_accounts_for_nested_original_decoder_scratch(){
        let raw=vec![0u8;4];let data=legaia_lzs::compress(&raw);let mut bundle=vec![];bundle.extend(1u32.to_le_bytes());bundle.extend(4u32.to_le_bytes());bundle.extend(4u32.to_le_bytes());bundle.extend(16u32.to_le_bytes());bundle.extend(data);
        let p=analyze(&bundle,None).unwrap();assert_eq!(p.events[0].destination_allocation_bytes,71680);assert_eq!(p.events[0].lzs_scratch_bytes,4116);assert_eq!(p.peak_accounted_requests,75796);assert_eq!(p.persistent_destination_requests,0);assert!(!p.live_heap_capacity_verified&&!p.native_gameplay_verified);
    }
    #[test]
    fn stream_single_tim_uses_source_without_temporary_allocation(){
        let compressed=legaia_lzs::compress(&[0u8;4]);
        let mut bundle=vec![];for w in [2u32,4,4,24,(20<<24)|4,24+compressed.len()as u32]{bundle.extend(w.to_le_bytes());}
        bundle.extend(compressed);bundle.extend([0u8;4]);
        let mut pac=vec![];for w in [4u32,0,0]{pac.extend(w.to_le_bytes());}
        let p=analyze(&bundle,Some(&pac)).unwrap();
        assert_eq!(p.events[2].carrier,"PAC chunk 0x0");
        assert!(!p.events[2].compressed);
        assert_eq!(p.events[2].destination_allocation_bytes,0);
        assert_eq!(p.events[2].lzs_scratch_bytes,0);
    }
}
