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
use std::collections::{HashMap, HashSet};

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

static SENT: MainCell<Option<HashSet<u32>>> = MainCell::new(None);
static RECORDS: MainCell<u64> = MainCell::new(0);
/// The records sent, by each device texture they name: a freed texture takes them with it
/// (`drain_freed_textures`). A handle in a list can be stale (its record went with another
/// texture): it goes only if it is still sent, and then it names this texture.
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

pub unsafe fn last_modelled(t: &MainThread) -> bool {
    *LAST_FLAGS.get(t) & (PROJECTED_LIGHTS | PROJECTED_SHADOWS) == 0
}

#[track_caller]
fn sent<'a>(t: &'a MainThread) -> Open<'a, HashSet<u32>> {
    SENT.get(t).map(|sent| sent.get_or_insert_with(HashSet::new))
}

/// Forgets the records that were sent: for a capture file from the stream.
pub unsafe fn reset(t: &MainThread) {
    sent(t).clear();
    by_texture(t).clear();
}

/// Forgets these records: the next draw with each content sends it again.
pub unsafe fn forget(t: &MainThread, handles: &[u32]) {
    for handle in handles {
        sent(t).remove(handle);
    }
}

#[track_caller]
fn by_texture<'a>(t: &'a MainThread) -> Open<'a, HashMap<u32, Vec<u32>>> {
    BY_TEXTURE.get(t).map(|by_texture| by_texture.get_or_insert_with(HashMap::new))
}

/// The records that name a device texture that was freed go: each gets a `MATERIAL_GONE`
/// record, and a later draw with the same content sends it again. A texture whose address a
/// new texture took before this drain keeps its records (`own_host::take_freed_textures`).
pub unsafe fn drain_freed_textures(t: &MainThread) {
    let freed = crate::own_host::take_freed_textures();
    for texture in freed {
        let Some(handles) = by_texture(t).remove(&texture) else { continue };
        for handle in handles {
            if sent(t).remove(&handle) {
                crate::capture::material_gone(handle);
                *GONE.get(t) += 1;
            }
        }
    }
}

pub unsafe fn report(t: &MainThread) -> String {
    format!("material: {} records, {} distinct, {} gone with their textures", *RECORDS.get(t), sent(t).len(), *GONE.get(t))
}

/// The device texture of a texture of the scene graph, from the texture manager of the
/// renderer (slot 2 of its table: `PrepareTextureForRendering`). 0 without a texture.
unsafe fn device_texture(renderer: usize, texture: usize) -> u32 {
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

/// A hash of the words, one multiply and one rotation for each word: the record is hashed
/// at every draw.
fn hash(words: &[u32]) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for &w in words {
        h = (h ^ w).wrapping_mul(0x9E37_79B1).rotate_left(13);
    }
    h.max(1)
}

/// Notes the material of a shape that is about to be drawn: sends its record when the
/// content is new. Returns the handle (the hash), or 0 without a property state.
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

    let handle = hash(&w);
    if sent(t).insert(handle) {
        crate::capture::material(handle, &w);
        *RECORDS.get(t) += 1;
        for at in TEXTURE_WORDS {
            if w[at] != 0 {
                by_texture(t).entry(w[at]).or_default().push(handle);
            }
        }
    }
    handle
}
