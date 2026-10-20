//! The draw of a shape as a record of facts, for the retained pipeline: the geometry handle,
//! the material handle, the world matrix and the lights of the draw. The helper makes the
//! state of the draw from the geometry and material records.
//!
//! The draw functions of the renderer send the record for a draw that has both handles and a
//! material the helper models. The helper draws from the records only. When the device is
//! the one of the plugin (`own_host`), the pipeline of the device does not run for such a
//! draw. With a Direct3D device behind the link, the pipeline runs too, for the picture of
//! the game.

use crate::capture;
use crate::mem::rd;

/// The flags of a record. `RELATIVE`: the world matrix is relative to the camera. Do not use
/// the flag 2 again: old capture files have it.
pub const RELATIVE: u32 = 1;
/// The shape is skinned: the record has the palette of its bones after the lights.
pub const SKINNED: u32 = 4;
/// The record is a particle system: the particles follow the lights, and the geometry
/// handle is the key of the particle data. See `tag::SHAPE`.
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
/// The record is the draw of lines (`RenderLines`): its geometry has the vertices
/// and the index list of the segments, and the helper draws it as a line list.
pub const LINES: u32 = 64;
/// The draw goes into the picture of the character of the inventory, and the Morrowind Code
/// Patch has its draw function there: with the blend on it draws the shape in a way of its
/// own. See `code_patch_picture`.
pub const CODE_PATCH_PICTURE: u32 = 128;
/// The record is the twin of the `INSTANCE_DRAW` record before it (the dual mode of
/// `instance_stream`): the helper draws one of the two.
pub const RETAINED_TWIN: u32 = 0x200;
/// The record ends with the world bound of the shape (the centre and the radius, 16 bytes,
/// after everything else): the helper sorts a blended draw by it among the blends of its
/// table. Every record of a draw has it.
pub const BOUND: u32 = 0x400;
/// The draw is a draw of the flush of the engine's main sorted queue (`ports::base::
/// in_main_flush`): the engine sorted it among its blends. A blend without the flag draws
/// in place. The head of the stream says that the records carry the flag
/// (`capture::HEADER_QUEUE_MARKED`); an older capture has no such word.
pub const QUEUED: u32 = 0x800;

/// The address of the flags of the Code Patch for the picture of the character, 1 when the
/// game has no such patch, 0 before the first look.
static CODE_PATCH_FLAGS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// True while the game draws the picture of the character of the inventory with the Code
/// Patch. `ui_updateCharacterImage` calls `WorldControllerRenderTarget::draw` 0x42DE10 at
/// 0x5CD38C. The Code Patch puts a function of its own there. That function sets a byte to
/// 1, makes the call and sets the byte to 0 (`C6 05 address 01`, then the call).
///
/// The Code Patch's `patchDrawIndexedPrimitive` reads the byte. For a draw with the blend
/// on, it draws with the alpha of the texture (the blend SRCALPHA and INVSRCALPHA, the alpha
/// test "greater than 0", the alpha of stage 0 from its first argument). Then, for a
/// material with an alpha that is not 1, it draws into the alpha channel only (the blend ONE
/// and ZERO, the alpha of stage 0 modulated). The game puts the number of the equipment slot
/// into the alpha of the material of each part. It reads the alpha of the picture back to
/// find the part under the pointer. The helper makes the same draws for a record with
/// `CODE_PATCH_PICTURE`.
pub unsafe fn code_patch_picture() -> bool {
    use core::sync::atomic::Ordering;
    const CALL: usize = 0x5CD38C;
    const DRAW: usize = 0x42DE10;
    let mut address = CODE_PATCH_FLAGS.load(Ordering::Relaxed);
    if address == 0 {
        address = 1;
        if rd::<u8>(CALL, 0) == 0xE8 {
            let target = (CALL + 5).wrapping_add(rd::<i32>(CALL, 1) as usize);
            if target != DRAW && rd::<u16>(target, 0) == 0x05C6 && rd::<u8>(target, 6) == 1 {
                address = rd::<u32>(target, 2) as usize;
                crate::log!("shapes: the Code Patch draws the picture of the character, its flags are at {address:#x}");
            }
        }
        CODE_PATCH_FLAGS.store(address, Ordering::Relaxed);
    }
    address > 1 && rd::<u8>(address, 0) & 1 != 0
}

/// The fog of a draw as `NiDX8RenderState::updateFogStates` and `updateD3DState` set it
/// for its fog property: `None` when the fog is off for the draw (the property is off, or
/// its mode is not one of the two the device fog has); else the colour, the start, the end
/// (the bits of the floats) and the range fog flag.
pub unsafe fn fog_values(renderer: usize, property_state: usize) -> Option<[u32; 4]> {
    use crate::gen::layouts::NiPropertyState as PS;
    fog_values_of(renderer, rd(property_state, PS::propFog))
}

/// The fog words of one fog property (the cell's, or a shape's own), with the render
/// state of the renderer for the range.
pub unsafe fn fog_values_of(renderer: usize, fog: usize) -> Option<[u32; 4]> {
    use crate::gen::layouts::{NiDX8RenderState as RS, NiDX8Renderer as R, NiFogProperty};
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

/// The handles of the object of the draw in progress, from the object record.
static mut HANDLES: (u32, u32) = (0, 0);
static mut RECORDS: u64 = 0;
/// The draws that got no record while the stream recorded, by the reason: no geometry
/// handle, no material handle. The helper does not draw them. The last count is the draws
/// of an empty geometry (no active vertex, no triangle): those have nothing to draw.
static mut WITHOUT: [u64; 3] = [0; 3];
/// The records of a material with projected lights or shadows: the helper draws the shape
/// without them.
static mut PROJECTED: u64 = 0;

/// True when the shape records go out: a stream or a capture file records.
pub fn on() -> bool {
    capture::recording()
}

/// True when the records are the only draws: the device is the one of the plugin, and the
/// helper draws the frames from the stream. The pipeline of the device does not run for a
/// shape then, and the ports make no device buffer for a geometry (the records carry the
/// vertices). The partitions of a skin are still made: the records need them.
pub fn records_only() -> bool {
    capture::recording() && crate::own_host::active()
}

/// The draw kind, the class and the geometry data of the object of the draws that follow,
/// for the log of a draw without a record.
static mut OBJECT: (u32, u32, usize) = (0, 0, 0);

/// Notes the handles of the object of the draws that follow, with its draw kind, its class
/// and its geometry data.
pub unsafe fn note_handles(geometry: u32, material: u32, kind: u32, class: u32, geometry_data: usize) {
    *core::ptr::addr_of_mut!(HANDLES) = (geometry, material);
    *core::ptr::addr_of_mut!(OBJECT) = (kind, class, geometry_data);
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
    let without = *core::ptr::addr_of!(WITHOUT);
    format!(
        "shapes: {} records, {} of them without their projected lights or shadows; draws without a record: {} with no geometry, {} with no material, {} skinned with no palette, {} skinned in software; {} draws of an empty geometry",
        *core::ptr::addr_of!(RECORDS),
        *core::ptr::addr_of!(PROJECTED),
        without[0],
        without[1],
        *core::ptr::addr_of!(WITHOUT_PALETTE),
        *core::ptr::addr_of!(SOFTWARE_SKINNED),
        without[2]
    )
}

/// The skinned draws that got no record because their skin has no complete palette.
static mut WITHOUT_PALETTE: u64 = 0;
/// The skinned draws that the game skins in software (MWSE's flag of the geometry, or less
/// than 4 bones of the device): `RenderShape` gives them to `DrawPrimitive`, which sends no
/// record.
static mut SOFTWARE_SKINNED: u64 = 0;

/// Counts a skinned draw that the game skins in software, and writes the first ones to the
/// log with the reason and the picture flag of the Code Patch.
pub unsafe fn software_skinned(why: &str) {
    if !on() {
        return;
    }
    let count = &mut *core::ptr::addr_of_mut!(SOFTWARE_SKINNED);
    *count += 1;
    if *count <= 8 {
        crate::log!("shapes: a skinned draw that the game skins in software ({why}), picture of the character {}", code_patch_picture());
    }
}

/// Counts a skinned draw that gets no record because the palette of its skin cannot be made
/// (`camera_space::skin_palette`), and writes the first ones to the log with the reason.
pub unsafe fn without_palette(why: &str) {
    if !on() {
        return;
    }
    let count = &mut *core::ptr::addr_of_mut!(WITHOUT_PALETTE);
    *count += 1;
    if *count <= 8 {
        let (kind, class, _) = *core::ptr::addr_of!(OBJECT);
        crate::log!("shapes: a skinned draw without a record: kind {kind}, class {class}: {why}");
    }
}

/// Counts a draw that gets no record while the stream records. The log gets the first ones
/// without a geometry that is not empty: the kind and the class of the object, and the
/// reason.
unsafe fn without(reason: usize) {
    if !on() {
        return;
    }
    let counts = &mut *core::ptr::addr_of_mut!(WITHOUT);
    if reason != 0 {
        counts[reason] += 1;
        return;
    }
    let (kind, class, geometry_data) = *core::ptr::addr_of!(OBJECT);
    let (why, empty) = crate::geometry_stream::why_none(kind, geometry_data);
    if empty {
        counts[2] += 1;
        return;
    }
    counts[0] += 1;
    if counts[0] <= 8 {
        crate::log!("shapes: a draw without a geometry record: kind {kind}, class {class}: {why}");
    }
}

/// Sends the record of the draw in progress when the object has both handles and the
/// stream records. `world` is the current world matrix of the renderer, after the transform of the
/// draw was set, with the camera-relative mask of the plugin; for a skinned shape `palette`
/// has the matrix of each bone of the skin instead. Returns true when the ports of the
/// pipeline must not run for this draw.
pub unsafe fn send(renderer: usize, world: [f32; 16], relative: bool, ambient: u32, material_values: [u32; 14], fog: Option<[u32; 4]>, palette: &[[f32; 16]]) -> bool {
    let (geometry, _) = *core::ptr::addr_of!(HANDLES);
    if geometry == 0 {
        without(0);
        return false;
    }
    // The bones as the memory of the palette: little-endian floats, as the stream has them
    // (`capture::words`).
    let bones = core::slice::from_raw_parts(palette.as_ptr() as *const u8, core::mem::size_of_val(palette));
    let flags = if palette.is_empty() { 0 } else { SKINNED };
    record(renderer, geometry, flags, world, relative, ambient, material_values, fog, palette.len() as u32, &[], bones)
}

/// Sends the record of the lines that are about to be drawn (`RenderLines`), with the flag
/// `LINES`; the geometry is the lines data, noted by the link. Returns true when the
/// pipeline of the device must not run for the draw.
pub unsafe fn send_lines(renderer: usize, world: [f32; 16], relative: bool, ambient: u32, material_values: [u32; 14], fog: Option<[u32; 4]>) -> bool {
    let (geometry, _) = *core::ptr::addr_of!(HANDLES);
    if geometry == 0 {
        without(0);
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
        without(0);
        return false;
    }
    let tail = [count, radius.to_bits(), right[0].to_bits(), right[1].to_bits(), right[2].to_bits(), up[0].to_bits(), up[1].to_bits(), up[2].to_bits()];
    record(renderer, key, PARTICLES, world, relative, ambient, material_values, fog, 0, &tail, particles)
}

/// The stages of the texture effects of the draw in progress (the environment map and the
/// fog map of the effect state of the renderer), as the texture pass of the device would
/// make them for this draw: the coordinate generation, the transform flags and the texture
/// matrix of each, from the effect and the camera of the frame; `None` when the draw has no
/// such effect. The material record has the effects' textures and modes; these values
/// change with the camera, so they come with each draw, like the world matrix.
unsafe fn effect_words(renderer: usize) -> Option<[u32; EFFECT_WORDS]> {
    use crate::gen::layouts::{NiDX8Renderer as R, NiDynamicEffectState as DES};
    use crate::ports::texture_pass_b::{EFFECT_STAGE_WORDS, effect_stage_words};
    let effect_state: usize = rd(renderer, R::currentEffectState);
    if effect_state == 0 {
        return None;
    }
    let environment: usize = rd(effect_state, DES::environmentMap);
    let fog_map: usize = rd(effect_state, DES::fogMap);
    if environment == 0 && fog_map == 0 {
        return None;
    }
    let mut words = [0u32; EFFECT_WORDS];
    words[..EFFECT_STAGE_WORDS].copy_from_slice(&effect_stage_words(renderer, environment));
    words[EFFECT_STAGE_WORDS..].copy_from_slice(&effect_stage_words(renderer, fog_map));
    if words.iter().all(|&w| w == 0) {
        return None;
    }
    Some(words)
}

/// The words of the stages of the two texture effects of a draw.
const EFFECT_WORDS: usize = 2 * crate::ports::texture_pass_b::EFFECT_STAGE_WORDS;

/// The most words of the tail of a record that the stack holds: the effects, the fog, and
/// the particle words.
const TAIL_WORDS: usize = EFFECT_WORDS + 4 + 8;

/// The record itself: the 36 words of the head, the keys of the lights, the stages of the
/// texture effects when the draw has any, the fog when the draw has it, then `tail` and
/// `bytes` (the bones, or the particle words and the particles).
unsafe fn record(renderer: usize, geometry: u32, kind: u32, world: [f32; 16], relative: bool, ambient: u32, material_values: [u32; 14], fog: Option<[u32; 4]>, bone_count: u32, tail: &[u32], bytes: &[u8]) -> bool {
    let (_, material) = *core::ptr::addr_of!(HANDLES);
    if !on() {
        return false;
    }
    if material == 0 {
        without(1);
        return false;
    }
    if !crate::material_stream::last_modelled() {
        *core::ptr::addr_of_mut!(PROJECTED) += 1;
    }
    let lights = crate::light_stream::note(renderer);
    let count = lights.len() as u32;
    let effects = effect_words(renderer);
    let mut flags = (relative as u32)
        | kind
        | if effects.is_none() { 0 } else { EFFECTS }
        | if fog.is_some() { FOG } else { 0 }
        | if code_patch_picture() { CODE_PATCH_PICTURE } else { 0 }
        | if crate::ports::base::in_main_flush() { QUEUED } else { 0 };
    // A draw of a retained shape: its instance records in the place of this one (mode gpu:
    // the first draw of the instance), or before it (mode dual).
    if let Some((shape, id)) = crate::instance_stream::take_pending() {
        // A shape (kind 0) or a skinned shape (its palette in `bytes`); not particles, lines.
        // A draw with texture effects stays (the table makes their stages from the material
        // record; `reject_reason` keeps the kinds the table cannot draw).
        if kind & !SKINNED != 0 || flags & CODE_PATCH_PICTURE != 0 {
            crate::instance_stream::reject_pending(shape, true);
        } else if crate::instance_stream::draw(shape, id, &world, relative, &material_values, fog, ambient, lights, if kind & SKINNED != 0 { bytes } else { &[] }, bone_count) {
            if crate::instance_stream::mode() == crate::instance_stream::Mode::Gpu {
                return records_only();
            }
            flags |= RETAINED_TWIN;
        }
    }
    // The tail of the record: the effects, the fog, then `tail`. On the stack: a draw does not
    // allocate for it (a longer tail than the stack holds goes to the heap).
    let parts: [&[u32]; 3] = [effects.as_ref().map_or(&[], |w| &w[..]), fog.as_ref().map_or(&[], |f| &f[..]), tail];
    let length: usize = parts.iter().map(|p| p.len()).sum();
    let mut stack = [0u32; TAIL_WORDS];
    let mut heap = Vec::new();
    let all: &mut [u32] = if length <= TAIL_WORDS {
        &mut stack[..length]
    } else {
        heap.resize(length, 0);
        &mut heap
    };
    let mut at = 0;
    for part in parts {
        all[at..at + part.len()].copy_from_slice(part);
        at += part.len();
    }
    let tail = &all[..];
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
    // The world bound the renderer holds for this draw (the ports copy it before the record
    // goes out: `RenderShape`, `RenderPoints` and `RenderLines` each set it first).
    let bound: [u32; 4] = rd(renderer, crate::gen::layouts::NiDX8Renderer::worldBound);
    words[2] |= BOUND;
    capture::shape(&words, lights, tail, bytes, &bound);
    *core::ptr::addr_of_mut!(RECORDS) += 1;
    records_only()
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
