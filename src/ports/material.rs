//! `NiDX8MaterialManager` keeps one `D3DMATERIAL8` for each `NiMaterialProperty`. It builds the
//! `D3DMATERIAL8` again when the revision of the property changes.

use crate::engine;
use crate::gen::layouts::{NiDX8MaterialManager as MM, NiDX8MaterialManager_Entry as E, NiMaterialProperty as MP};
use crate::mem::{rd, wr, x87_pass};

/// `NiDX8MaterialManager::Entry` holds a `D3DMATERIAL8` and the revision that the game built it from.
pub const ENTRY_SIZE: usize = E::SIZE;
const ENTRY_REVISION: usize = E::revisionID;

// D3DMATERIAL8
const DIFFUSE: usize = E::d3dMaterial_Diffuse;
const AMBIENT: usize = E::d3dMaterial_Ambient;
const SPECULAR: usize = E::d3dMaterial_Specular;
const EMISSIVE: usize = E::d3dMaterial_Emissive;
const POWER: usize = E::d3dMaterial_power;

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

/// The 6 bytes of the `SetMaterial` call in `UpdateToDevice`. The Morrowind Code Patch
/// ("MCP-011") puts a call of its function `patchSetMaterialAlpha` there.
const UPDATE_SET_MATERIAL_CALL: usize = 0x6C2696;
const PATCH_SET_MATERIAL_ALPHA: usize = 0x736600;
/// `patchdataRenderTargetFlags` of the Morrowind Code Patch, one byte.
const PATCH_RENDER_TARGET_FLAGS: usize = 0x77445B;

/// True when the game code calls `patchSetMaterialAlpha` in the place of `SetMaterial`.
unsafe fn code_patch_material_alpha() -> bool {
    rd::<u8>(UPDATE_SET_MATERIAL_CALL, 0) == 0xE8
        && (UPDATE_SET_MATERIAL_CALL + 5).wrapping_add(rd::<u32>(UPDATE_SET_MATERIAL_CALL, 1) as usize) == PATCH_SET_MATERIAL_ALPHA
}

/// 0x6C2680 `NiDX8MaterialManager::UpdateToDevice`: sends the current material to the device
/// when it changed.
///
/// With the Morrowind Code Patch the function also keeps a flag for the patch: bit 1 is on
/// after a material that is not opaque, and only bit 0 stays after an opaque one. The port
/// does that when the call of the patch is in the game code.
pub unsafe fn update_to_device(this: usize, device: usize) {
    crate::shadow::touch(&update_to_device_port::STATE);
    if rd::<u8>(this, MM::changed) == 0 {
        return;
    }
    wr::<u8>(this, MM::changed, 0);
    let entry: usize = rd(this, MM::currentMaterialEntry);
    if code_patch_material_alpha() {
        // The patch compares the bits of the diffuse alpha with the bits of 1.0.
        let flags: u8 = rd(PATCH_RENDER_TARGET_FLAGS, 0);
        let opaque = rd::<u32>(entry, DIFFUSE + 12) == 0x3F80_0000;
        wr::<u8>(PATCH_RENDER_TARGET_FLAGS, 0, if opaque { flags & 1 } else { flags | 2 });
    }
    crate::d3d::device_set_material(device, entry);
}

unsafe fn update_to_device_regions(set: &mut crate::shadow::Regions, this: usize) {
    set.add(this, MM::SIZE);
    set.add(PATCH_RENDER_TARGET_FLAGS, 1);
}

/// The call is a skip when the material has no entry and the allocator hooks are not active.
/// The call allocates the entry then.
unsafe fn set_current_material_regions(set: &mut crate::shadow::Regions, this: usize, material: usize) {
    set.add(this, MM::SIZE);
    set.add(material, MP::SIZE);
    let entry: usize = rd(material, MP::rendererData);
    if entry == 0 {
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

port_thiscall!(update_to_device_port, 0x6C2680, "NiDX8MaterialManager::UpdateToDevice",
    fn(this, device: usize) -> (),
    compare_eax = false,
    regions = |set| update_to_device_regions(set, this),
    rust = update_to_device);

pub static PORTS: &[&crate::shadow::PortState] =
    &[&entry_update_port::STATE, &set_current_material_port::STATE, &update_to_device_port::STATE];

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
