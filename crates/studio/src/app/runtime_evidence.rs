//! Raw runtime observations can exclude placements, never certify free VRAM,
//! native ownership, heap headroom or source-image identity.
use anyhow::{Result,ensure};
use super::texture_relocation::Rect;

pub(in crate::app) struct VramObservations {
    activity:Vec<bool>,
    pub sources:Vec<(String,String)>,
    pub changed_words:usize,
}
impl VramObservations {
    pub fn from_raw(snapshots:&[(&str,&[u8])])->Result<Self> {
        ensure!(!snapshots.is_empty()&&snapshots.len()<=16,"Expected 1–16 bounded raw VRAM snapshots");
        let mut result=Self{activity:vec![false;1024*512],sources:Vec::new(),changed_words:0};
        let first=snapshots[0].1;ensure!(first.len()==1024*512*2,"Raw VRAM must contain 1024x512 little-endian halfwords");
        let mut changed=vec![false;1024*512];
        for &(label,raw) in snapshots {
            ensure!(!label.is_empty()&&label.len()<=128&&!label.chars().any(char::is_control)&&!result.sources.iter().any(|(s,_)|s==label),"Invalid/duplicate observation label");
            ensure!(raw.len()==first.len(),"Raw VRAM snapshot size mismatch");
            result.sources.push((label.into(),super::super::collision_recipes::digest(raw)));
            for (i,word) in raw.chunks_exact(2).enumerate() {
                let differs=word!=&first[i*2..i*2+2];
                changed[i]|=differs;
                // Conservative hazard union: nonzero may be stale; a zero
                // word may still be live. Neither classification grants use.
                result.activity[i]|=word!=[0,0]||differs;
            }
        }
        result.changed_words=changed.iter().filter(|v|**v).count();Ok(result)
    }
    pub fn intersects(&self,r:Rect)->Result<bool> {
        ensure!(r.w>0&&r.h>0&&r.x as u32+r.w as u32<=1024&&r.y as u32+r.h as u32<=512,"Observation query outside PS1 VRAM");
        Ok((r.y..r.y+r.h).any(|y|(r.x..r.x+r.w).any(|x|self.activity[y as usize*1024+x as usize])))
    }
    pub fn mask(&self)->&[bool]{&self.activity}
    pub fn summary(&self)->String {
        format!("{} raw VRAM observations, {} activity/hazard words, {} changed words. Nonzero may be stale, zero/unseen remains UNKNOWN, not free. No runtime or source-image acceptance is granted.",self.sources.len(),self.activity.iter().filter(|v|**v).count(),self.changed_words)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cleared_observed_data_stays_reserved_and_zeros_do_not_certify_freedom() {
        let a=vec![0;1024*512*2];let mut b=a.clone();let at=(64*1024+880)*2;b[at]=7;
        let observation=VramObservations::from_raw(&[("pre",&a),("during",&b),("after",&a)]).unwrap();
        assert!(observation.intersects(Rect{x:880,y:64,w:16,h:64}).unwrap());assert_eq!(observation.changed_words,1);
        assert!(!observation.intersects(Rect{x:900,y:200,w:1,h:1}).unwrap());assert!(observation.summary().contains("not free"));
        assert!(VramObservations::from_raw(&[("bad",&a[..10])]).is_err());assert!(VramObservations::from_raw(&[("same",&a),("same",&a)]).is_err());
    }
}
