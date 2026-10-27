//! The material of a draw as a record of facts, for the retained pipeline.
//!
//! The record has the values of the properties of the shape (texturing with its maps, alpha,
//! material, vertex colour, z-buffer, stencil, fog, wireframe, shade, dither, specular) and
//! of its dynamic effects (the environment map, the fog map, the counts of projected lights
//! and shadows). The helper derives the state of the draw from these facts.
//!
//! A record goes out once for each distinct content (a hash of the words). The texture of a
//! map is the device texture that the texture manager gives for it, the same handle that
//! the state packet has.

use core::mem::transmute;
use std::collections::HashMap;

use crate::ptr_hash::HashSet;

use crate::checked::Open;
use crate::main_thread::{MainCell, MainThread};
use crate::engine;
use crate::gen::layouts;
use crate::gen::layouts::{
    NiAlphaProperty as AP, NiDX8Renderer as R, NiDynamicEffectState as DES, NiDynamicEffectState_InternalList as LIST,
    NiPropertyState as PS, NiStencilProperty as SP, NiTexturingProperty as NTP,
    NiTexturingProperty_BumpMap as BUMP, NiTexturingProperty_Map as MAP, NiVertexColorProperty as VC, NiZBufferProperty as ZP,
};
use crate::mem::rd;

/// The number of words of a record: 91 for the properties and the textures and modes of the
/// effects, then 13 for each texture effect that do not change with the camera
/// (`texture_pass_b::effect_static_words`: the environment map at 91, the fog map at 104),
/// for the stages of the table of the helper.
pub const WORDS: usize = 117;
const NI_TEXTURE_RENDERER_DATA: usize = layouts::NiTexture::rendererData;
const SOURCE_DATA_D3D_TEXTURE: usize = layouts::NiDX8SourceTextureData::d3dTexture;
const SOURCE_TEXTURE_IS_STATIC: usize = layouts::NiSourceTexture::isStatic;
/// The table of `NiDX8SourceTextureData`: a rendered texture has another renderer data.
const VTBL_SOURCE_TEXTURE_DATA: u32 = 0x74F770;
/// The flags of a property (u16).
const PROPERTY_FLAGS: usize = layouts::NiProperty::flags;
/// The fields of a `NiTextureEffect` (see `texture_pass_b`).
const EFFECT_ENABLED: usize = layouts::NiTextureEffect::enabled;
const EFFECT_TEXTURE: usize = layouts::NiTextureEffect::sourceTexture;
const EFFECT_FILTER_MODE: usize = layouts::NiTextureEffect::filterMode;
const EFFECT_CLAMP_MODE: usize = layouts::NiTextureEffect::clampMode;
const EFFECT_COORDINATE_MODE: usize = layouts::NiTextureEffect::coordMode;

// The bits of word 1.
pub const BLEND: u32 = 1;
pub const ALPHA_TEST: u32 = 2;
pub const DEPTH_TEST: u32 = 4;
pub const DEPTH_WRITE: u32 = 8;
pub const WIREFRAME: u32 = 16;
pub const SMOOTH: u32 = 32;
pub const DITHER: u32 = 64;
pub const SPECULAR: u32 = 128;
pub const FOG: u32 = 256;
pub const STENCIL: u32 = 512;
pub const ENVIRONMENT_MAP: u32 = 1024;
pub const FOG_MAP: u32 = 2048;
pub const PROJECTED_LIGHTS: u32 = 4096;
pub const PROJECTED_SHADOWS: u32 = 8192;
/// The fog property of the shape is its own, not the cell's: words 78 and 79 hold its
/// colour and its density, 54 and 55 its start and end as the render state computes them.
pub const OWN_FOG: u32 = 16384;
/// The alpha property has the no-sorter bit (bit 13): the engine's accumulator draws the
/// blend at once, in the order of the scene, and the table draws it with the opaque.
pub const NO_SORTER: u32 = 32768;

/// The handles given to a content, for the session: the check hash of the content
/// ([`hash`]) and whether its record is sent now. A handle names one content for the whole
/// session: a second content with the same handle steps to the next free handle
/// (`handle_of`), and a handle whose record went (`MATERIAL_GONE`) keeps its content. The
/// helper keeps the first content of a handle.
static HANDLES: MainCell<Option<HashMap<u32, Handle>>> = MainCell::new(None);
/// The steps past the handle of another content: one for each note of a content whose
/// first handle another content has (a collision of the first hash).
static COLLISIONS: MainCell<u64> = MainCell::new(0);

#[derive(Clone, Copy)]
struct Handle {
    check: u32,
    sent: bool,
    /// The record went with a freed texture (`MATERIAL_GONE`): [`GONE_UNAGED`] until a
    /// sweep notes the frame, then the frame of that sweep; 0 while it is not gone.
    gone_since: u32,
}

/// A gone record whose age the next sweep starts.
const GONE_UNAGED: u32 = u32::MAX;

/// The frames between two sweeps of the handles ([`sweep`]), and the least age of a gone
/// handle that leaves the table.
pub const SWEEP_FRAMES: u32 = 1024;
/// The handles the sweeps took out of the table.
static SWEPT: MainCell<u64> = MainCell::new(0);

/// A hash of each group of the words of each sent record ([`GROUP_NAMES`]), made when the
/// record goes and dropped when it is unsent: what a new note of an instance changed.
static SENT_GROUPS: MainCell<Option<HashMap<u32, [u32; GROUPS]>>> = MainCell::new(None);

#[track_caller]
fn sent_groups<'a>(t: &'a MainThread) -> Open<'a, HashMap<u32, [u32; GROUPS]>> {
    SENT_GROUPS.get(t).map(|groups| groups.get_or_insert_with(HashMap::new))
}

/// The groups of the words of a record, for the count of what a note of an instance finds
/// changed (`instance_stream`, the note of every 64th frame).
pub const GROUPS: usize = 11;
pub const GROUP_NAMES: [&str; GROUPS] = [
    "flags and alpha",
    "texturing and map modes",
    "a map's texture",
    "bump",
    "vertex colour",
    "z-buffer and stencil",
    "own fog",
    "an effect's texture",
    "an effect's modes",
    "projected lights and shadows",
    "an effect's projection",
];

/// The group of a word of the record (the layout of [`note`]).
fn group_of(word: usize) -> usize {
    match word {
        8 | 13 | 18 | 23 | 28 | 33 | 38 | 43 => 2,
        6..=47 => 1,
        48..=53 => 3,
        54 | 55 | 78..=80 => 6,
        68 | 69 => 4,
        70..=77 => 5,
        81 | 85 => 7,
        82..=84 | 86..=88 => 8,
        89 | 90 => 9,
        91.. => 10,
        // The size, the flags, alpha, and the words 56 to 67 that stay 0.
        _ => 0,
    }
}

/// A hash of each group of the words (FNV-1a).
fn group_hashes(words: &[u32]) -> [u32; GROUPS] {
    let mut hashes = [0x811C_9DC5u32; GROUPS];
    for (index, &word) in words.iter().enumerate() {
        let hash = &mut hashes[group_of(index)];
        *hash = (*hash ^ word).wrapping_mul(0x0100_0193);
    }
    hashes
}

/// The groups whose hashes differ, as bits (bit `g` for [`GROUP_NAMES`]`[g]`).
fn differing_groups(a: &[u32; GROUPS], b: &[u32; GROUPS]) -> u16 {
    (0..GROUPS).filter(|&g| a[g] != b[g]).fold(0, |bits, g| bits | 1 << g)
}

/// The groups of words in which the records of two sent handles differ, as bits; [`None`]
/// when one of them is not sent now.
pub fn changed_groups(t: &MainThread, old: u32, new: u32) -> Option<u16> {
    let groups = sent_groups(t);
    Some(differing_groups(groups.get(&old)?, groups.get(&new)?))
}
static RECORDS: MainCell<u64> = MainCell::new(0);
/// The records sent, by each device texture they name (its serial): a freed texture takes them
/// with it (`drain_freed_textures`). A handle in a list can be stale (its record went with
/// another texture): it goes only if it is still sent, and then it names this texture (a
/// handle names one content for the session, [`HANDLES`]).
static BY_TEXTURE: MainCell<Option<HashMap<u32, Vec<u32>>>> = MainCell::new(None);
/// The `MATERIAL_GONE` records sent.
static GONE: MainCell<u64> = MainCell::new(0);
/// The words of a record that hold a device texture: the 8 maps, the environment map, the
/// fog map.
const TEXTURE_WORDS: [usize; 10] = [8, 13, 18, 23, 28, 33, 38, 43, 81, 85];
/// The flags of the last record noted: the helper models a material without projected
/// lights and shadows.
static LAST_FLAGS: MainCell<u32> = MainCell::new(0);

/// The shape's own shade, dither and specular properties from its NIF go into the record
/// (see `note`). Off by default: the game never showed them. `nirender.nifShading(on)`.
static NIF_SHADING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// True when the record takes the shape's own shade, dither and specular properties.
pub fn nif_shading() -> bool {
    NIF_SHADING.load(core::sync::atomic::Ordering::Relaxed)
}

pub fn set_nif_shading(on: bool) {
    NIF_SHADING.store(on, core::sync::atomic::Ordering::Relaxed);
}

/// # Safety
///
/// No condition of its own: the function reads no engine memory. `t` is the proof of the main
/// thread.
pub unsafe fn last_modelled(t: &MainThread) -> bool {
    *LAST_FLAGS.get(t) & (PROJECTED_LIGHTS | PROJECTED_SHADOWS) == 0
}

#[track_caller]
fn handles<'a>(t: &'a MainThread) -> Open<'a, HashMap<u32, Handle>> {
    HANDLES.get(t).map(|handles| handles.get_or_insert_with(HashMap::new))
}

/// Takes the record of a handle off the sent ones. True when it was sent.
fn unsend(t: &MainThread, handle: u32) -> bool {
    sent_groups(t).remove(&handle);
    handles(t).get_mut(&handle).is_some_and(|h| core::mem::replace(&mut h.sent, false))
}

/// Forgets the records that were sent: for a capture file from the stream. The handles keep
/// their contents.
///
/// # Safety
///
/// No condition of its own: the function reads no engine memory. `t` is the proof of the main
/// thread.
pub unsafe fn reset(t: &MainThread) {
    for h in handles(t).values_mut() {
        h.sent = false;
    }
    sent_groups(t).clear();
    by_texture(t).clear();
}

/// Forgets these records: the next draw with each content sends it again.
///
/// # Safety
///
/// No condition of its own: the function reads no engine memory. `t` is the proof of the main
/// thread.
pub unsafe fn forget(t: &MainThread, handles: &[u32]) {
    for &handle in handles {
        unsend(t, handle);
    }
}

/// The handle of a content with the hash `first` and the check hash `check`, and whether its
/// record must go now. The first free handle from `first` on, or the one that has this
/// content.
fn handle_of(t: &MainThread, first: u32, check: u32) -> (u32, bool) {
    let mut table = handles(t);
    let mut handle = first;
    loop {
        match table.get_mut(&handle) {
            Some(h) if h.check == check => {
                h.gone_since = 0;
                return (handle, !core::mem::replace(&mut h.sent, true));
            }
            Some(_) => {
                *COLLISIONS.get(t) += 1;
                handle = handle.wrapping_add(1).max(1);
            }
            None => {
                table.insert(handle, Handle { check, sent: true, gone_since: 0 });
                return (handle, true);
            }
        }
    }
}

#[track_caller]
fn by_texture<'a>(t: &'a MainThread) -> Open<'a, HashMap<u32, Vec<u32>>> {
    BY_TEXTURE.get(t).map(|by_texture| by_texture.get_or_insert_with(HashMap::new))
}

/// The records that name a device texture that was freed go: each gets a `MATERIAL_GONE`
/// record, and a later draw with the same content sends it again. A new texture at the same
/// address has another serial (`own_host::serial_of`), so its records are others. The table's
/// instances that use one of them note their material again at their next display
/// (`instance_stream::note_materials_again`).
///
/// # Safety
///
/// No condition of its own: the function reads no engine memory. `t` is the proof of the main
/// thread.
pub unsafe fn drain_freed_textures(t: &MainThread) {
    let freed = crate::own_host::take_freed_textures();
    let mut gone = Vec::new();
    for texture in freed {
        let Some(materials) = by_texture(t).remove(&texture) else { continue };
        for handle in materials {
            if unsend(t, handle) {
                crate::capture::material_gone(handle);
                if let Some(h) = handles(t).get_mut(&handle) {
                    h.gone_since = GONE_UNAGED;
                }
                *GONE.get(t) += 1;
                gone.push(handle);
            }
        }
    }
    if !gone.is_empty() {
        crate::instance_stream::note_materials_again(t, &gone);
    }
}

/// The handles of records that went with their textures leave the table when no instance
/// names them (`in_use`) and a sweep at least [`SWEEP_FRAMES`] frames ago found them gone.
/// Their content names a freed texture's serial, which never comes again, so without this
/// the table grows over a session. The helper has dropped such a record by then: it drops a
/// gone material at its next scan when no instance names it, within 64 frames. Only then can
/// another content take the handle without the helper keeping the old one. A record unsent
/// by the start of a capture (`reset`) never got a `MATERIAL_GONE`, so it stays.
pub fn sweep(t: &MainThread, in_use: &HashSet<u32>, frame: u32) {
    let removed = sweep_table(&mut handles(t), in_use, frame);
    *SWEPT.get(t) += removed;
}

fn sweep_table(table: &mut HashMap<u32, Handle>, in_use: &HashSet<u32>, frame: u32) -> u64 {
    let before = table.len();
    table.retain(|handle, h| {
        if h.gone_since == 0 || h.sent || in_use.contains(handle) {
            return true;
        }
        if h.gone_since == GONE_UNAGED {
            h.gone_since = frame.max(1);
            return true;
        }
        frame.wrapping_sub(h.gone_since) < SWEEP_FRAMES
    });
    (before - table.len()) as u64
}

/// # Safety
///
/// No condition of its own: the function reads no engine memory. `t` is the proof of the main
/// thread.
pub unsafe fn report(t: &MainThread) -> String {
    let (sent, all) = {
        let table = handles(t);
        (table.values().filter(|h| h.sent).count(), table.len())
    };
    format!(
        "material: {} records, {} distinct sent now, {} handles in the session, {} gone with their textures ({} of their handles swept), {} steps past the handle of another content (collisions of the first hash, at each note)",
        *RECORDS.get(t),
        sent,
        all,
        *GONE.get(t),
        *SWEPT.get(t),
        *COLLISIONS.get(t)
    )
}

/// The device texture of a texture of the scene graph, from the texture manager of the
/// renderer (slot 2 of its table: `PrepareTextureForRendering`), as the stream names it (its
/// serial, `own_host::serial_of`). 0 without a texture.
unsafe fn device_texture(renderer: usize, texture: usize) -> u32 {
    crate::capture::stream_handle(device_texture_address(renderer, texture))
}

/// The address of the device texture of a texture of the scene graph (see `device_texture`).
unsafe fn device_texture_address(renderer: usize, texture: usize) -> u32 {
    if texture == 0 {
        return 0;
    }
    // A static texture that is loaded has its device texture in its renderer data
    // (`NiDX8SourceTextureData::d3dTexture`): no call of the manager for it. A texture that
    // is not static goes to the manager at each draw, as the engine's draw does: its update
    // copies a new revision of the pixel data (a mod that changes the pixels), and returns at
    // once when there is none.
    let data: usize = rd(texture, NI_TEXTURE_RENDERER_DATA);
    if data != 0 && rd::<u32>(data, 0) == VTBL_SOURCE_TEXTURE_DATA {
        let loaded: u32 = rd(data, SOURCE_DATA_D3D_TEXTURE);
        if loaded != 0 && rd::<u8>(texture, SOURCE_TEXTURE_IS_STATIC) != 0 {
            return loaded;
        }
    }
    let manager: usize = rd(renderer, R::textureManager);
    let (mut created, mut mipmapped) = (0u32, 0u32);
    let prepare: unsafe extern "thiscall" fn(usize, usize, *mut u32, *mut u32) -> usize = transmute(rd::<usize>(rd::<usize>(manager, 0), 8));
    prepare(manager, texture, &mut created, &mut mipmapped) as u32
}

/// Two hashes of the words, one multiply and one rotation for each word in each: the record
/// is hashed at every draw. The first gives the handle; the second, with other constants, is
/// the check that tells two contents with the same first hash apart (`handle_of`). The two
/// run side by side in one pass.
fn hash(words: &[u32]) -> (u32, u32) {
    let (mut h, mut c): (u32, u32) = (0x811C_9DC5, 0x2545_F491);
    for &w in words {
        h = (h ^ w).wrapping_mul(0x9E37_79B1).rotate_left(13);
        c = (c ^ w).wrapping_mul(0x85EB_CA77).rotate_left(17);
    }
    (h.max(1), c)
}

/// Notes the material of a shape that is about to be drawn: sends its record when the
/// content is new. Returns the handle (the hash), or 0 without a property state.
///
/// # Safety
///
/// `renderer` points at the live `NiDX8Renderer`; `property_state` is 0 or a live `NiPropertyState`
/// whose properties are each 0 or live, with live maps and textures; `effect_state` is 0 or a live
/// dynamic effect state whose effects are each 0 or live. The function reads at most 64 effects of
/// each list.
pub unsafe fn note(t: &MainThread, renderer: usize, property_state: usize, effect_state: usize) -> u32 {
    if !crate::capture::recording() || property_state == 0 {
        return 0;
    }
    let flags_of = |property: usize| if property != 0 { rd::<u16>(property, PROPERTY_FLAGS) as u32 } else { 0 };
    let mut w = [0u32; WORDS];
    w[0] = WORDS as u32;
    let mut flags = 0;

    // Alpha.
    let alpha: usize = rd(property_state, PS::propAlpha);
    let af = flags_of(alpha);
    if af & 1 != 0 {
        flags |= BLEND;
    }
    if (af >> 9) & 1 != 0 {
        flags |= ALPHA_TEST;
    }
    if (af >> 13) & 1 != 0 {
        flags |= NO_SORTER;
    }
    w[2] = (af >> 1) & 0xF;
    w[3] = (af >> 5) & 0xF;
    w[4] = (af >> 10) & 7;
    w[5] = if alpha != 0 { rd::<u8>(alpha, AP::alphaTestRef) as u32 } else { 0 };

    // Texturing: the apply mode, the decal count as the pipeline counts it, the maps.
    let texturing: usize = rd(property_state, PS::propTexturing);
    if texturing != 0 {
        w[6] = rd(texturing, NTP::applyMode);
        let maps: usize = rd(texturing, NTP::maps_storage);
        let end: u32 = rd(texturing, NTP::maps_endIndex);
        let mut decals = end.wrapping_sub(6);
        if decals == 1 && crate::ports::ni::texturing_map(maps, crate::ports::ni::MAP_DECAL_0) == 0 {
            decals = 0;
        }
        w[7] = if end >= 6 { decals.min(2) } else { 0 };
        for i in 0..8usize {
            let map: usize = if (i as u32) < end { rd(maps, i * 4) } else { 0 };
            let at = 8 + i * 5;
            if map != 0 {
                let texture: usize = rd(map, MAP::texture);
                w[at] = device_texture(renderer, texture);
                w[at + 1] = rd(map, MAP::clampMode);
                w[at + 2] = rd(map, MAP::filterMode);
                w[at + 3] = rd(map, MAP::texCoordSet);
                w[at + 4] = (texture != 0) as u32;
                if i == 5 {
                    w[48] = rd(map, BUMP::lumaScale);
                    w[49] = rd(map, BUMP::lumaOffset);
                    for k in 0..4 {
                        w[50 + k] = rd(map, BUMP::bumpMat + k * 4);
                    }
                }
            }
        }
    }

    // The colours, the shininess and the alpha of the material are in the shape record:
    // controllers change them for each frame, and the hash of this record must not change
    // with them. Words 54 to 67 stay 0.

    // Vertex colour.
    let vertex_color: usize = rd(property_state, PS::propVertexColor);
    if vertex_color != 0 {
        w[68] = rd(vertex_color, VC::sourceMode);
        w[69] = rd(vertex_color, VC::lightingMode);
    }

    // Z-buffer.
    let z: usize = rd(property_state, PS::propZBuffer);
    let zf = flags_of(z);
    if zf & 1 != 0 {
        flags |= DEPTH_TEST;
    }
    if zf & 2 != 0 {
        flags |= DEPTH_WRITE;
    }
    w[70] = if z != 0 { rd(z, ZP::testFunction) } else { 0 };

    // Stencil.
    let stencil: usize = rd(property_state, PS::propStencil);
    if stencil != 0 {
        if rd::<u8>(stencil, SP::enabled) != 0 {
            flags |= STENCIL;
        }
        for (k, field) in [SP::testFunc, SP::reference, SP::mask, SP::failAction, SP::zFailAction, SP::passAction, SP::drawMode].into_iter().enumerate() {
            w[71 + k] = rd(stencil, field);
        }
    }

    // Fog.
    let fog: usize = rd(property_state, PS::propFog);
    let ff = flags_of(fog);
    if (ff >> 1) & 1 != 0 {
        flags |= FOG;
    }
    // The density and the colour of the cell's fog change with the weather at each frame:
    // they are not in the record (the helper takes that fog from the render states). A fog
    // property of the shape's own is static: its colour, density, start and end go in.
    if fog != 0 {
        w[80] = (ff >> 3) & 3;
        if fog != engine::game_fog_property() {
            if let Some(values) = crate::shape_stream::fog_values_of(renderer, fog) {
                flags |= OWN_FOG;
                w[78] = values[0];
                w[79] = rd(fog, crate::gen::layouts::NiFogProperty::density);
                w[54] = values[1];
                w[55] = values[2];
            }
        }
    }

    // Wireframe, shade, dither, specular. The engine takes the last three from its global
    // properties; the shape's own (from its NIF) sit in the slots of its property state
    // (`setProperty` stores a property at `propAlpha + 4 * type`: dither 8, specular 9,
    // shade 10) and nothing of the engine reads them. With `nif_shading` on, the record
    // takes the shape's own when it has one.
    if flags_of(rd(property_state, PS::propWireframe)) & 1 != 0 {
        flags |= WIREFRAME;
    }
    let own_or_global = |slot: usize, global: usize| -> usize {
        let own: usize = if nif_shading() { rd(property_state, slot) } else { 0 };
        if own != 0 {
            own
        } else {
            global
        }
    };
    if flags_of(own_or_global(0x30, engine::property_state_global_shade_property(property_state))) & 1 != 0 {
        flags |= SMOOTH;
    }
    if flags_of(own_or_global(0x28, engine::property_state_global_dither_property(property_state))) & 1 != 0 {
        flags |= DITHER;
    }
    if flags_of(own_or_global(0x2C, engine::property_state_global_specular_property(property_state))) & 1 != 0 {
        flags |= SPECULAR;
    }

    // The dynamic effects.
    if effect_state != 0 {
        let effect_words = |effect: usize, at: usize, static_at: usize, w: &mut [u32; WORDS]| -> bool {
            if effect == 0 || rd::<u8>(effect, EFFECT_ENABLED) == 0 {
                return false;
            }
            let texture: usize = rd(effect, EFFECT_TEXTURE);
            if texture == 0 {
                return false;
            }
            w[at] = device_texture(renderer, texture);
            w[at + 1] = rd(effect, EFFECT_CLAMP_MODE);
            w[at + 2] = rd(effect, EFFECT_FILTER_MODE);
            w[at + 3] = rd(effect, EFFECT_COORDINATE_MODE);
            // The world projection of the effect, for the stage of a retained draw.
            let r#static = crate::ports::texture_pass_b::effect_static_words(renderer, effect);
            w[static_at..static_at + r#static.len()].copy_from_slice(&r#static);
            true
        };
        if effect_words(rd(effect_state, DES::environmentMap), 81, 91, &mut w) {
            flags |= ENVIRONMENT_MAP;
        }
        if effect_words(rd(effect_state, DES::fogMap), 85, 104, &mut w) {
            flags |= FOG_MAP;
        }
        let count = |head: usize| {
            let mut n = 0u32;
            let mut node = head;
            while node != 0 && n < 64 {
                let effect: usize = rd(node, LIST::effect);
                if effect != 0 && rd::<u8>(effect, EFFECT_ENABLED) != 0 && rd::<usize>(effect, EFFECT_TEXTURE) != 0 {
                    n += 1;
                }
                node = rd(node, LIST::next);
            }
            n
        };
        w[89] = count(rd(effect_state, DES::projLights));
        w[90] = count(rd(effect_state, DES::projShadows));
        if w[89] != 0 {
            flags |= PROJECTED_LIGHTS;
        }
        if w[90] != 0 {
            flags |= PROJECTED_SHADOWS;
        }
    }
    w[1] = flags;
    *LAST_FLAGS.get(t) = flags;

    let (first, check) = hash(&w);
    let (handle, new) = handle_of(t, first, check);
    if new {
        crate::capture::material(handle, &w);
        sent_groups(t).insert(handle, group_hashes(&w));
        *RECORDS.get(t) += 1;
        for at in TEXTURE_WORDS {
            if w[at] != 0 {
                by_texture(t).entry(w[at]).or_default().push(handle);
            }
        }
    }
    handle
}

/// The words of a texture effect that [`note`] copies into a material record, with the
/// source texture in place of its device texture: the state, the texture, the modes and the
/// world projection. Equal words give the same material; all zeros for an effect that gives
/// no stage.
pub const EFFECT_SIGNATURE_WORDS: usize = 4 + crate::ports::texture_pass_b::EFFECT_STATIC_WORDS;

/// See [`EFFECT_SIGNATURE_WORDS`].
///
/// # Safety
///
/// `renderer` points at the live `NiDX8Renderer`; `effect` is 0 or a live `NiTextureEffect`.
pub unsafe fn effect_signature(renderer: usize, effect: usize) -> [u32; EFFECT_SIGNATURE_WORDS] {
    let mut words = [0u32; EFFECT_SIGNATURE_WORDS];
    if effect == 0 || rd::<u8>(effect, EFFECT_ENABLED) == 0 {
        return words;
    }
    let texture: usize = rd(effect, EFFECT_TEXTURE);
    if texture == 0 {
        return words;
    }
    words[0] = texture as u32;
    words[1] = rd(effect, EFFECT_CLAMP_MODE);
    words[2] = rd(effect, EFFECT_FILTER_MODE);
    words[3] = rd(effect, EFFECT_COORDINATE_MODE);
    words[4..].copy_from_slice(&crate::ports::texture_pass_b::effect_static_words(renderer, effect));
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(sent: bool, gone_since: u32) -> Handle {
        Handle { check: 1, sent, gone_since }
    }

    #[test]
    fn a_gone_handle_leaves_only_when_unused_and_old_enough() {
        let mut table: HashMap<u32, Handle> = HashMap::default();
        table.insert(1, handle(false, GONE_UNAGED));
        table.insert(2, handle(false, GONE_UNAGED));
        table.insert(3, handle(true, 0));
        table.insert(4, handle(false, 0));
        let in_use: HashSet<u32> = [2u32].into_iter().collect();
        // The first sweep starts the age of 1; 2 is named by an instance; 3 is sent; 4 was
        // unsent by a capture's start, never gone.
        assert_eq!(sweep_table(&mut table, &in_use, 100), 0);
        assert_eq!(sweep_table(&mut table, &in_use, 100 + SWEEP_FRAMES - 1), 0);
        assert_eq!(sweep_table(&mut table, &in_use, 100 + SWEEP_FRAMES), 1);
        assert!(!table.contains_key(&1));
        assert!(table.contains_key(&2) && table.contains_key(&3) && table.contains_key(&4));
    }

    #[test]
    fn the_groups_follow_the_layout_of_the_record() {
        assert_eq!(GROUP_NAMES[group_of(8)], "a map's texture");
        assert_eq!(GROUP_NAMES[group_of(9)], "texturing and map modes");
        assert_eq!(GROUP_NAMES[group_of(81)], "an effect's texture");
        assert_eq!(GROUP_NAMES[group_of(89)], "projected lights and shadows");
        assert_eq!(GROUP_NAMES[group_of(91)], "an effect's projection");
        assert_eq!(GROUP_NAMES[group_of(WORDS - 1)], "an effect's projection");
    }

    #[test]
    fn a_changed_word_shows_in_its_group_only() {
        let words = [3u32; WORDS];
        let mut other = words;
        other[13] = 4;
        let bits = differing_groups(&group_hashes(&words), &group_hashes(&other));
        assert_eq!(bits, 1 << 2);
        assert_eq!(differing_groups(&group_hashes(&words), &group_hashes(&words)), 0);
    }
}
