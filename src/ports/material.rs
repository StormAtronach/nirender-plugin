//! `NiDX8MaterialManager` keeps one `D3DMATERIAL8` for each `NiMaterialProperty`. It builds the
//! `D3DMATERIAL8` again when the revision of the property changes.

use crate::engine;
use crate::gen::layouts::{NiDX8MaterialManager as MM, NiMaterialProperty as MP};
use crate::mem::{rd, wr, x87_pass};

/// `NiDX8MaterialManager::Entry` holds a `D3DMATERIAL8` and the revision that the game built it from.
pub const ENTRY_SIZE: usize = 0x48;
const ENTRY_REVISION: usize = 0x44;

// D3DMATERIAL8
const DIFFUSE: usize = 0x00;
const AMBIENT: usize = 0x10;
const SPECULAR: usize = 0x20;
const EMISSIVE: usize = 0x30;
const POWER: usize = 0x40;

/// 0x6C2240 `NiDX8MaterialManager::Entry::Update`. The port returns 1 when it built the entry again.
pub unsafe fn entry_update(entry: usize, material: usize) -> u8 {
    crate::shadow::touch(&entry_update_port::STATE);
    let revision: u32 = rd(material, MP::revisionID);
    if rd::<u32>(entry, ENTRY_REVISION) == revision {
        return 0;
    }
    wr(entry, ENTRY_REVISION, revision);
    // The game code moves alpha and shininess through the FPU. It copies the colours without the FPU.
    let alpha = x87_pass(rd(material, MP::alpha));
    for (to, from) in [(EMISSIVE, MP::emissive), (AMBIENT, MP::ambient), (DIFFUSE, MP::diffuse), (SPECULAR, MP::specular)] {
        wr::<u32>(entry, to, rd(material, from));
        wr::<u32>(entry, to + 4, rd(material, from + 4));
        wr::<u32>(entry, to + 8, rd(material, from + 8));
        wr::<u32>(entry, to + 12, alpha);
    }
    wr::<u32>(entry, POWER, x87_pass(rd(material, MP::shininess)));
    1
}

/// 0x6C2620 `NiDX8MaterialManager::SetCurrentMaterial`
pub unsafe fn set_current_material(this: usize, material: usize) {
    crate::shadow::touch(&set_current_material_port::STATE);
    let mut entry: usize = rd(material, MP::rendererData);
    if entry == 0 {
        entry = engine::operator_new(ENTRY_SIZE);
        if entry != 0 {
            wr::<u32>(entry, ENTRY_REVISION, 0);
            entry_update(entry, material);
        }
        wr(material, MP::rendererData, entry);
    }
    if entry_update(entry, material) != 0 {
        wr::<u8>(this, MM::changed, 1);
    }
    if entry != rd::<usize>(this, MM::currentMaterialEntry) {
        wr::<u8>(this, MM::changed, 1);
        wr(this, MM::currentMaterialEntry, entry);
    }
}

/// The call is a skip when the material has no entry and the allocator hooks are not active.
/// The call allocates the entry then.
unsafe fn set_current_material_regions(set: &mut crate::shadow::Regions, this: usize, material: usize) {
    set.add(this, MM::SIZE);
    set.add(material, MP::SIZE);
    let entry: usize = rd(material, MP::rendererData);
    if entry == 0 {
        // This call makes the entry.
        set.allocates();
    }
    set.add(entry, ENTRY_SIZE);
}

port_thiscall!(entry_update_port, 0x6C2240, "NiDX8MaterialManager::Entry::Update",
    fn(this, material: usize) -> u8,
    compare_eax = true,
    regions = |set| set.add(this, ENTRY_SIZE),
    rust = entry_update);

port_thiscall!(set_current_material_port, 0x6C2620, "NiDX8MaterialManager::SetCurrentMaterial",
    fn(this, material: usize) -> (),
    compare_eax = false,
    regions = |set| set_current_material_regions(set, this, material),
    rust = set_current_material);

pub static PORTS: &[&crate::shadow::PortState] = &[&entry_update_port::STATE, &set_current_material_port::STATE];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_update_builds_the_d3d_material() {
        let mut material = [0u8; MP::SIZE];
        let mut entry = [0u8; ENTRY_SIZE];
        let (m, e) = (material.as_mut_ptr() as usize, entry.as_mut_ptr() as usize);
        unsafe {
            for (i, base) in [MP::ambient, MP::diffuse, MP::specular, MP::emissive].into_iter().enumerate() {
                for c in 0..3 {
                    wr::<f32>(m, base + c * 4, (i * 10 + c) as f32);
                }
            }
            wr::<f32>(m, MP::alpha, 0.5);
            wr::<f32>(m, MP::shininess, 25.0);
            wr::<u32>(m, MP::revisionID, 7);

            assert_eq!(entry_update(e, m), 1);
            assert_eq!(rd::<[f32; 4]>(e, DIFFUSE), [10.0, 11.0, 12.0, 0.5]);
            assert_eq!(rd::<[f32; 4]>(e, AMBIENT), [0.0, 1.0, 2.0, 0.5]);
            assert_eq!(rd::<[f32; 4]>(e, SPECULAR), [20.0, 21.0, 22.0, 0.5]);
            assert_eq!(rd::<[f32; 4]>(e, EMISSIVE), [30.0, 31.0, 32.0, 0.5]);
            assert_eq!(rd::<f32>(e, POWER), 25.0);
            assert_eq!(rd::<u32>(e, ENTRY_REVISION), 7);
            // The revision is the same, so the port does nothing, also when the colours changed.
            wr::<f32>(m, MP::alpha, 1.0);
            assert_eq!(entry_update(e, m), 0);
            assert_eq!(rd::<f32>(e, DIFFUSE + 12), 0.5);
        }
    }

    #[test]
    fn set_current_material_marks_a_change_of_entry() {
        let mut material = [0u8; MP::SIZE];
        let mut entry = [0u8; ENTRY_SIZE];
        let mut manager = [0u8; MM::SIZE];
        let (m, e, mm) = (material.as_mut_ptr() as usize, entry.as_mut_ptr() as usize, manager.as_mut_ptr() as usize);
        unsafe {
            wr(m, MP::rendererData, e);
            wr::<u32>(m, MP::revisionID, 0); // The entry has this revision, so the port does not build it.
            set_current_material(mm, m);
            assert_eq!(rd::<usize>(mm, MM::currentMaterialEntry), e);
            assert_eq!(rd::<u8>(mm, MM::changed), 1);
            wr::<u8>(mm, MM::changed, 0);
            set_current_material(mm, m);
            assert_eq!(rd::<u8>(mm, MM::changed), 0);
            wr::<u32>(m, MP::revisionID, 3);
            set_current_material(mm, m);
            assert_eq!(rd::<u8>(mm, MM::changed), 1);
        }
    }
}
