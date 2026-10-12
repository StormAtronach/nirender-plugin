//! The draw of a shape as a record of facts, for the retained pipeline (step R3): the
//! geometry handle, the material handle, the world matrix and the lights of the draw. The
//! helper makes the state of the draw from the geometry and material records.
//!
//! `RenderShape` and `RenderTriStrips` send the record for a shape that has both handles and
//! a material the helper models, and do not call the pipeline of the device for it. In the
//! dual mode they call the pipeline too, and the helper takes one of the two draws by its
//! switch: the two can then be compared on one capture.

use crate::capture;
use crate::mem::rd;

/// The modes of `nirender.shapes(mode)`.
pub const OFF: u32 = 0;
pub const DUAL: u32 = 1;
pub const ONLY: u32 = 2;

/// The flags of a record.
pub const RELATIVE: u32 = 1;
pub const DUAL_DRAW: u32 = 2;
/// The shape is skinned: the record has the palette of its bones after the lights.
pub const SKINNED: u32 = 4;

static mut MODE: u32 = DUAL;
/// The handles of the object of the draw in progress, from the object record.
static mut HANDLES: (u32, u32) = (0, 0);
static mut RECORDS: u64 = 0;
static mut SKIPPED: u64 = 0;

pub unsafe fn set_mode(mode: u32) {
    *core::ptr::addr_of_mut!(MODE) = mode.min(ONLY);
}

pub unsafe fn mode() -> u32 {
    *core::ptr::addr_of!(MODE)
}

/// Notes the handles of the object of the draws that follow.
pub unsafe fn note_handles(geometry: u32, material: u32) {
    *core::ptr::addr_of_mut!(HANDLES) = (geometry, material);
}

pub unsafe fn report() -> String {
    format!("shapes: mode {}, {} records, {} draws of the device skipped", mode(), *core::ptr::addr_of!(RECORDS), *core::ptr::addr_of!(SKIPPED))
}

/// Sends the record of the draw in progress when the object has both handles and the mode
/// is on. `world` is the current world matrix of the renderer, after the transform of the
/// draw was set, with the camera-relative mask of the plugin; for a skinned shape `palette`
/// has the matrix of each bone of the skin instead. Returns true when the ports of the
/// pipeline must not run for this draw.
pub unsafe fn send(renderer: usize, world: [f32; 16], relative: bool, ambient: u32, material_values: [u32; 14], palette: &[[f32; 16]]) -> bool {
    let mode = mode();
    let (geometry, material) = *core::ptr::addr_of!(HANDLES);
    if mode == OFF || geometry == 0 || material == 0 || !capture::recording() || !crate::material_stream::last_modelled() {
        return false;
    }
    let lights = crate::link_client::state_lights(renderer);
    let count = (lights.len() / crate::link_client::STATE_LIGHT_SIZE) as u32;
    let flags = (relative as u32) | if mode == DUAL { DUAL_DRAW } else { 0 } | if palette.is_empty() { 0 } else { SKINNED };
    let mut words = [0u32; 36];
    words[0] = geometry;
    words[1] = material;
    words[2] = flags;
    words[3] = count;
    words[4] = ambient;
    for (i, value) in world.iter().enumerate() {
        words[5 + i] = value.to_bits();
    }
    // The colours of the material: ambient, diffuse, specular, emissive, shininess, alpha.
    words[21..35].copy_from_slice(&material_values);
    words[35] = palette.len() as u32;
    let bones: Vec<u8> = palette.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
    capture::shape(&words, lights, &bones);
    *core::ptr::addr_of_mut!(RECORDS) += 1;
    if mode == ONLY {
        *core::ptr::addr_of_mut!(SKIPPED) += 1;
        return true;
    }
    false
}

/// The colours of the material of a property state, for `send`: ambient, diffuse, specular,
/// emissive (3 each), shininess, alpha, as the bits of the floats.
pub unsafe fn material_values(property_state: usize) -> [u32; 14] {
    use crate::gen::layouts::NiMaterialProperty as MP;
    let material = crate::engine::property_state_update_current_material_revision(property_state);
    let mut values = [0u32; 14];
    if material != 0 {
        for k in 0..12 {
            values[k] = rd(material, MP::ambient + k * 4);
        }
        values[12] = rd(material, MP::shininess);
        values[13] = rd(material, MP::alpha);
    }
    values
}

/// The world matrix of the renderer, for `send`.
pub unsafe fn current_world(renderer: usize) -> [f32; 16] {
    rd(renderer, crate::gen::layouts::NiDX8Renderer::currentWorldMatrix)
}
