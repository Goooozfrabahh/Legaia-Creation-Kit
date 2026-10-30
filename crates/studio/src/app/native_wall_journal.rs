//! Ownership of serialized editor wall writes, never nearby retail geometry.
//! Recomposition preserves the baseline and every surviving owner's layer.
use super::collision_recipes::digest;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Reference {
    pub cell: [u8; 2],
    pub descriptor: u16,
    pub model_selector: u16,
    pub descriptor_sha256: String,
    pub model_sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Owner {
    pub id: String,
    pub blocked: Vec<[i32; 2]>,
    pub open: Vec<[i32; 2]>,
    /// Evidence labels must describe actual serialized writes, not inferred
    /// native ownership. Native reference witnesses are supplied by callers.
    pub evidence: String,
    pub native: Option<Reference>,
    #[serde(default)]
    pub mesh_policy: Option<[f32; 2]>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    pub schema: u32,
    pub scene: String,
    pub map_entry: u32,
    pub source_map_sha256: String,
    /// High four collision bits only. Low floor-tier nibbles are independent.
    pub baseline_high: Vec<u8>,
    pub owners: Vec<Owner>,
}

fn check_map(map: &[u8]) -> Result<()> {
    ensure!(map.len() == 0x12000, "Expected complete native MAP");
    Ok(())
}
fn address([x, z]: [i32; 2]) -> Result<(usize, u8)> {
    ensure!(
        (0..256).contains(&x) && (0..256).contains(&z),
        "Wall cell out of bounds"
    );
    Ok((
        (z as usize / 2) * 128 + x as usize / 2,
        1 << (4 + (z as usize % 2) * 2 + x as usize % 2),
    ))
}

/// Capture precisely the cells the existing unrestricted wall writer emits.
/// Rasterization supplies NEW authored geometry, not ownership of old walls.
pub(super) fn serialized_owner(
    id: String,
    recipe: &super::collision_recipes::Recipe,
    draw: &super::world_view::DrawInfo,
) -> Result<Owner> {
    let mut blocked = if let Some(g) = &recipe.generated {
        g.wall_cells(
            [0, 1, 2].map(|a| draw.center[a] - g.pivot[a]),
            draw.rotation,
            draw.scale,
        )
        .map_err(anyhow::Error::msg)?
    } else {
        Default::default()
    };
    let (dx, dz) = if recipe.walls.is_empty() && recipe.nonblocking.is_empty() {
        (0, 0)
    } else {
        ensure!(
            draw.offset[0].is_finite() && draw.offset[2].is_finite(),
            "Non-finite wall translation"
        );
        let dx = draw.offset[0].round() as i32;
        let dz = draw.offset[2].round() as i32;
        ensure!(
            dx % 64 == 0 && dz % 64 == 0,
            "Manual collision cells require exact 64-unit translation"
        );
        (dx / 64, dz / 64)
    };
    for &[x, z] in &recipe.walls {
        blocked.insert([x + dx, z + dz]);
    }
    let open = recipe
        .nonblocking
        .iter()
        .map(|&[x, z]| [x + dx, z + dz])
        .collect::<Vec<_>>();
    if recipe.generated.is_none() {
        ensure!(
            draw.rotation == [0.; 3] && draw.scale == [1.; 3] && draw.offset[1] == 0.,
            "Manual wall cell writer has no rotation/scale/vertical representation"
        );
    }
    for &cell in blocked.iter().chain(&open) {
        address(cell)?;
    }
    let mesh_policy = if recipe.walls.is_empty() && recipe.nonblocking.is_empty() {
        recipe
            .generated
            .as_ref()
            .map(|g| [g.band_height, g.min_face_area])
    } else {
        None
    };
    Ok(Owner {
        id,
        blocked: blocked.into_iter().collect(),
        open,
        evidence:
            "Exact replay of serialized editor wall-cell writes; not inferred retail ownership"
                .into(),
        native: None,
        mesh_policy,
    })
}
pub(super) fn apply_owner(map: &mut [u8], owner: &Owner) -> Result<()> {
    check_map(map)?;
    for &cell in &owner.open {
        let (at, mask) = address(cell)?;
        map[0x4000 + at] &= !mask;
    }
    for &cell in &owner.blocked {
        let (at, mask) = address(cell)?;
        map[0x4000 + at] |= mask;
    }
    Ok(())
}
impl Journal {
    fn high(&self) -> Result<Vec<u8>> {
        ensure!(
            self.schema == 1
                && self.baseline_high.len() == 0x4000
                && self.baseline_high.iter().all(|v| v & 15 == 0),
            "Invalid native wall baseline"
        );
        ensure!(self.owners.len() <= 4096, "Unbounded collision journal");
        let mut high = self.baseline_high.clone();
        let mut ids = std::collections::BTreeSet::new();
        for owner in &self.owners {
            ensure!(
                !owner.id.is_empty() && owner.id.len() <= 256 && ids.insert(&owner.id),
                "Duplicate/invalid collision owner"
            );
            ensure!(
                !owner.evidence.is_empty()
                    && owner.evidence.len() <= 2048
                    && owner.blocked.len() + owner.open.len() <= 131072,
                "Invalid serialized wall evidence"
            );
            // Same order as the native writer: explicit openings then blockers.
            for &cell in &owner.open {
                let (at, mask) = address(cell)?;
                high[at] &= !mask;
            }
            for &cell in &owner.blocked {
                let (at, mask) = address(cell)?;
                high[at] |= mask;
            }
        }
        Ok(high)
    }
    pub fn reconstruct(
        scene: String,
        map_entry: u32,
        baseline: &[u8],
        source: &[u8],
        owners: Vec<Owner>,
    ) -> Result<Self> {
        check_map(baseline)?;
        check_map(source)?;
        let journal = Self {
            schema: 1,
            scene,
            map_entry,
            source_map_sha256: digest(source),
            baseline_high: baseline[0x4000..0x8000].iter().map(|v| v & 0xf0).collect(),
            owners,
        };
        journal.validate(source)?;
        Ok(journal)
    }
    pub fn validate(&self, source: &[u8]) -> Result<()> {
        check_map(source)?;
        ensure!(
            digest(source) == self.source_map_sha256,
            "Collision journal belongs to another native MAP revision"
        );
        let high = self.high()?;
        ensure!(source[0x4000..0x8000].iter().zip(&high).all(|(a,b)|a&0xf0==*b),"Serialized contributions do not reconstruct the ENTIRE native wall grid; no ownership removal allowed");
        for owner in &self.owners {
            if let Some(r) = &owner.native {
                ensure!(
                    r.cell.iter().all(|c| *c < 128) && r.descriptor < 512,
                    "Invalid collision instance witness"
                );
                let at = 0x8000 + (r.cell[1] as usize * 128 + r.cell[0] as usize) * 2;
                let id = u16::from_le_bytes(source[at..at + 2].try_into()?) & 511;
                ensure!(id == r.descriptor, "Collision instance reference changed");
                let record =
                    legaia_scene_formats::field_objects::ObjectRecord::parse(source, id as usize)
                        .context("Collision instance descriptor")?;
                ensure!(
                    record.pack_index_field == r.model_selector
                        && digest(&source[id as usize * 32..id as usize * 32 + 32])
                            == r.descriptor_sha256,
                    "Collision instance descriptor changed"
                );
            }
        }
        Ok(())
    }
    /// Apply explicit owner replacements/removals to an exactly validated source.
    /// Shared bits are retained by recomputing all layers, not clearing a mesh's
    /// stencil from the finished grid. Floors and all other MAP bytes are opaque.
    pub fn revise(
        &self,
        source: &[u8],
        changes: &[(String, Option<Owner>)],
    ) -> Result<(Vec<u8>, Self)> {
        self.validate(source)?;
        let mut next = self.clone();
        let mut seen = std::collections::BTreeSet::new();
        for (id, replacement) in changes {
            ensure!(seen.insert(id), "Duplicate ownership change");
            let index = next
                .owners
                .iter()
                .position(|o| &o.id == id)
                .context("Unknown source collision owner; cannot infer native ownership")?;
            if let Some(owner) = replacement {
                ensure!(&owner.id == id, "Collision replacement identity changed");
                next.owners[index] = owner.clone();
            } else {
                next.owners.remove(index);
            }
        }
        next.compose_over(source)
    }
    /// Caller first validates source ownership and the full existing/new write
    /// replay. Preserve all non-wall fields in the final native MAP template.
    pub fn compose_over(&self, template: &[u8]) -> Result<(Vec<u8>, Self)> {
        check_map(template)?;
        let mut next = self.clone();
        let high = next.high()?;
        let mut output = template.to_vec();
        for (i, h) in high.into_iter().enumerate() {
            output[0x4000 + i] = (output[0x4000 + i] & 15) | h;
        }
        next.source_map_sha256 = digest(&output);
        next.validate(&output)?;
        Ok((output, next))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner(id: &str, cells: &[[i32; 2]]) -> Owner {
        Owner {
            id: id.into(),
            blocked: cells.to_vec(),
            open: vec![],
            evidence: "Exact test serialized writes".into(),
            native: None,
            mesh_policy: None,
        }
    }
    fn compose(base: &[u8], owners: Vec<Owner>) -> (Vec<u8>, Journal) {
        let j = Journal {
            schema: 1,
            scene: "cell".into(),
            map_entry: 1,
            source_map_sha256: String::new(),
            baseline_high: base[0x4000..0x8000].iter().map(|v| v & 0xf0).collect(),
            owners,
        };
        let mut m = base.to_vec();
        for (i, v) in j.high().unwrap().into_iter().enumerate() {
            m[0x4000 + i] = (m[0x4000 + i] & 15) | v;
        }
        let j = Journal::reconstruct("cell".into(), 1, base, &m, j.owners).unwrap();
        (m, j)
    }
    #[test]
    fn move_and_delete_preserve_shared_native_walls_floors_and_other_owners() {
        let mut base = vec![0x09; 0x12000];
        let (at, bit) = address([3, 3]).unwrap();
        base[0x4000 + at] |= bit;
        let (m, j) = compose(
            &base,
            vec![
                owner("original", &[[3, 3], [4, 4], [5, 5]]),
                owner("duplicate", &[[5, 5], [6, 6]]),
            ],
        );
        let (out, j) = j
            .revise(
                &m,
                &[("original".into(), Some(owner("original", &[[20, 20]])))],
            )
            .unwrap();
        for (cell, blocked) in [
            ([3, 3], true),
            ([4, 4], false),
            ([5, 5], true),
            ([6, 6], true),
            ([20, 20], true),
        ] {
            let (at, bit) = address(cell).unwrap();
            assert_eq!(out[0x4000 + at] & bit != 0, blocked);
        }
        for i in 0..out.len() {
            if (0x4000..0x8000).contains(&i) {
                assert_eq!(out[i] & 15, m[i] & 15);
            } else {
                assert_eq!(out[i], m[i]);
            }
        }
        let (out, j) = j.revise(&out, &[("original".into(), None)]).unwrap();
        j.validate(&out).unwrap();
        let (at, bit) = address([20, 20]).unwrap();
        assert_eq!(out[0x4000 + at] & bit, 0);
    }
    #[test]
    fn stale_or_incomplete_ownership_never_authorizes_clearing() {
        let base = vec![0; 0x12000];
        let (m, j) = compose(&base, vec![owner("one", &[[4, 4]])]);
        let mut stale = m.clone();
        stale[0x4000] ^= 0x10;
        assert!(j.revise(&stale, &[("one".into(), None)]).is_err());
        assert!(Journal::reconstruct("cell".into(), 1, &base, &m, vec![]).is_err());
        assert!(j.revise(&m, &[("nearby-unowned".into(), None)]).is_err());
    }
    #[test]
    fn serialized_journal_reopens_without_losing_owner_identity() {
        let base = vec![0; 0x12000];
        let (m, j) = compose(&base, vec![owner("foreign-instance", &[[9, 9]])]);
        let reopened: Journal = serde_json::from_slice(&serde_json::to_vec(&j).unwrap()).unwrap();
        reopened.validate(&m).unwrap();
        assert_eq!(reopened.owners, j.owners);
    }
}
