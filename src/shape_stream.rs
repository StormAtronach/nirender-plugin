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
/// The record is a particle system (step R7): the particles follow the lights, and the
/// geometry handle is the key of the particle data. See `tag::SHAPE`.
pub const PARTICLES: u32 = 8;
/// Bytes of one particle in the record: the position, the size, the colour.
pub const PARTICLE_BYTES: usize = 20;
/// The record has the stages of the texture effects of the draw after the lights: for the
/// environment map and then for the fog map, `EFFECT_STAGE_WORDS` each (zeros for an
/// effect the draw does not have). See `effect_words`.
pub const EFFECTS: u32 = 16;
/// The record has the fog of the draw after the effects: the colour, the start and the
/// end as `NiDX8RenderState::updateFogStates` computes them, and the range fog flag (4
/// words). The density and the colour of a fog property change with the weather, so they
/// come with each draw and not in the material record. See `fog_values`.
pub const FOG: u32 = 32;
/// The record is the draw of lines (`RenderLines`, step R7b): its geometry has the vertices
/// and the index list of the segments, and the helper draws it as a line list.
pub const LINES: u32 = 64;

/// The fog of a draw as `NiDX8RenderState::updateFogStates` and `updateD3DState` set it
/// for its fog property: `None` when the fog is off for the draw (the property is off, or
/// its mode is not one of the two the device fog has); else the colour, the start, the end
/// (the bits of the floats) and the range fog flag.
pub unsafe fn fog_values(renderer: usize, property_state: usize) -> Option<[u32; 4]> {
    use crate::gen::layouts::{NiDX8RenderState as RS, NiDX8Renderer as R, NiFogProperty, NiPropertyState as PS};
    let fog: usize = rd(property_state, PS::propFog);
    if fog == 0 {
        return None;
    }
    // `NiProperty::flags`: bit 1 is the fog on, bits 3 and 4 the mode (0 world Z, 1 range).
    let flags: u8 = rd(fog, 0x14);
    if (flags >> 1) & 1 == 0 || (flags >> 3) & 3 >= 2 {
        return None;
    }
    let render_state: usize = rd(renderer, R::renderState);
    let (start, end) = crate::x87::fog_range(
        rd(fog, NiFogProperty::density),
        rd(render_state, RS::frustumDistance),
        rd(render_state, RS::frustumFar),
        rd(render_state, RS::inverseMaxFogValue),
    );
    Some([rd(fog, NiFogProperty::color), start, end, ((flags >> 3) & 3 == 1) as u32])
}

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

/// True when the records are the only draws: the pipeline of the device does not run for a
/// shape, so the ports make no device buffer for a geometry (the records carry the
/// vertices). The partitions of a skin are still made: the records need them.
pub unsafe fn records_only() -> bool {
    mode() == ONLY && capture::recording()
}

/// Notes the handles of the object of the draws that follow.
pub unsafe fn note_handles(geometry: u32, material: u32) {
    *core::ptr::addr_of_mut!(HANDLES) = (geometry, material);
}

/// Notes the geometry of the draw in progress, for a draw whose geometry the port packs
/// itself (a screen polygon): the object record of the link had none.
pub unsafe fn note_geometry(geometry: u32) {
    (*core::ptr::addr_of_mut!(HANDLES)).0 = geometry;
}

/// The arguments of `NiDX8LightManager::setState` for the draw in progress: the light
/// manager, the effect state, the texturing and the vertex colour properties. Each draw
/// function of the renderer makes this call before its draw; a port makes it at its own
/// call site before the record.
pub unsafe fn light_state_arguments(renderer: usize) -> (usize, usize, usize, usize) {
    use crate::gen::layouts::{NiDX8Renderer as R, NiPropertyState as PS};
    let property_state: usize = rd(renderer, R::currentPropertyState);
    (rd(renderer, R::lightManager), rd(renderer, R::currentEffectState), rd(property_state, PS::propTexturing), rd(property_state, PS::propVertexColor))
}

/// `D3DRS_AMBIENT` (139) as the render state record of the renderer has it.
pub unsafe fn ambient(renderer: usize) -> u32 {
    use crate::gen::layouts::{NiDX8RenderState as RS, NiDX8Renderer as R};
    rd(rd::<usize>(renderer, R::renderState), RS::renderStateSettings + 139 * 8)
}

pub unsafe fn report() -> String {
    format!("shapes: mode {}, {} records, {} draws of the device skipped", mode(), *core::ptr::addr_of!(RECORDS), *core::ptr::addr_of!(SKIPPED))
}

/// Sends the record of the draw in progress when the object has both handles and the mode
/// is on. `world` is the current world matrix of the renderer, after the transform of the
/// draw was set, with the camera-relative mask of the plugin; for a skinned shape `palette`
/// has the matrix of each bone of the skin instead. Returns true when the ports of the
/// pipeline must not run for this draw.
pub unsafe fn send(renderer: usize, world: [f32; 16], relative: bool, ambient: u32, material_values: [u32; 14], fog: Option<[u32; 4]>, palette: &[[f32; 16]]) -> bool {
    let (geometry, _) = *core::ptr::addr_of!(HANDLES);
    if geometry == 0 {
        return false;
    }
    let bones: Vec<u8> = palette.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
    let flags = if palette.is_empty() { 0 } else { SKINNED };
    record(renderer, geometry, flags, world, relative, ambient, material_values, fog, palette.len() as u32, &[], &bones)
}

/// Sends the record of the lines that are about to be drawn (`RenderLines`), with the flag
/// `LINES`; the geometry is the lines data, noted by the link. Returns true when the
/// pipeline of the device must not run for the draw.
pub unsafe fn send_lines(renderer: usize, world: [f32; 16], relative: bool, ambient: u32, material_values: [u32; 14], fog: Option<[u32; 4]>) -> bool {
    let (geometry, _) = *core::ptr::addr_of!(HANDLES);
    if geometry == 0 {
        return false;
    }
    record(renderer, geometry, LINES, world, relative, ambient, material_values, fog, 0, &[], &[])
}

/// Sends the record of a particle system that is about to be drawn (`RenderPoints`), with
/// the flag `PARTICLES`. `key` is the particle data of the game; `right` and `up` are the
/// axes of the camera in the model space of the draw; `particles` has `PARTICLE_BYTES` for
/// each particle. Returns true when the pipeline of the device must not run for the draw.
pub unsafe fn send_particles(
    renderer: usize,
    key: u32,
    world: [f32; 16],
    relative: bool,
    ambient: u32,
    material_values: [u32; 14],
    fog: Option<[u32; 4]>,
    radius: f32,
    right: [f32; 3],
    up: [f32; 3],
    particles: &[u8],
) -> bool {
    let count = (particles.len() / PARTICLE_BYTES) as u32;
    if key == 0 || count == 0 {
        return false;
    }
    let tail = [count, radius.to_bits(), right[0].to_bits(), right[1].to_bits(), right[2].to_bits(), up[0].to_bits(), up[1].to_bits(), up[2].to_bits()];
    record(renderer, key, PARTICLES, world, relative, ambient, material_values, fog, 0, &tail, particles)
}

/// The stages of the texture effects of the draw in progress (the environment map and the
/// fog map of the effect state of the renderer), as the texture pass of the device would
/// make them for this draw: the coordinate generation, the transform flags and the texture
/// matrix of each, from the effect and the camera of the frame. Empty when the draw has no
/// such effect. The material record has the effects' textures and modes; these values
/// change with the camera, so they come with each draw, like the world matrix.
unsafe fn effect_words(renderer: usize) -> Vec<u32> {
    use crate::gen::layouts::{NiDX8Renderer as R, NiDynamicEffectState as DES};
    let effect_state: usize = rd(renderer, R::currentEffectState);
    if effect_state == 0 {
        return Vec::new();
    }
    let environment: usize = rd(effect_state, DES::environmentMap);
    let fog_map: usize = rd(effect_state, DES::fogMap);
    if environment == 0 && fog_map == 0 {
        return Vec::new();
    }
    let mut words = Vec::with_capacity(2 * crate::ports::texture_pass_b::EFFECT_STAGE_WORDS);
    for effect in [environment, fog_map] {
        words.extend_from_slice(&crate::ports::texture_pass_b::effect_stage_words(renderer, effect));
    }
    if words.iter().all(|&w| w == 0) {
        return Vec::new();
    }
    words
}

/// The record itself: the 36 words of the head, the keys of the lights, the stages of the
/// texture effects when the draw has any, the fog when the draw has it, then `tail` and
/// `bytes` (the bones, or the particle words and the particles).
unsafe fn record(renderer: usize, geometry: u32, kind: u32, world: [f32; 16], relative: bool, ambient: u32, material_values: [u32; 14], fog: Option<[u32; 4]>, bone_count: u32, tail: &[u32], bytes: &[u8]) -> bool {
    let mode = mode();
    let (_, material) = *core::ptr::addr_of!(HANDLES);
    if mode == OFF || material == 0 || !capture::recording() || !crate::material_stream::last_modelled() {
        return false;
    }
    let lights = crate::light_stream::note(renderer);
    let count = lights.len() as u32;
    let mut effects = effect_words(renderer);
    let flags = (relative as u32) | if mode == DUAL { DUAL_DRAW } else { 0 } | kind | if effects.is_empty() { 0 } else { EFFECTS } | if fog.is_some() { FOG } else { 0 };
    if let Some(fog) = fog {
        effects.extend_from_slice(&fog);
    }
    effects.extend_from_slice(tail);
    let tail = &effects[..];
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
    words[35] = bone_count;
    capture::shape(&words, lights, tail, bytes);
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
