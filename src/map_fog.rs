//! The fog of war of the local map, kept on the CPU (milestone 17, records entry 56).
//!
//! The engine keeps the fog of each map tile in a rendered texture of 64 x 64. The map menu
//! reads it through `WorldControllerRenderTarget::lockRenderTarget` 0x42F3F0: the engine's
//! `getFogOfWarPixel` 0x42FE20 for one pixel, and MWSE's cache of one compositor pass
//! (`getFogOfWarPixelCached`, on the two calls in `ui_MenuMap_isPositionUncoveredByFogOfWar`)
//! for all of a texture. On the plugin's device each lock sends the helper's frame to the GPU
//! and waits for it.
//!
//! This module keeps a CPU copy of each fog texture, as OpenMW keeps its fog on the CPU. A
//! lock of a texture with a copy gives the copy's pixels, with no read of the GPU; its unlock
//! lets them go. The engine still draws the fog on the GPU for the map menu. The copy follows
//! the two places where the engine draws the fog [V IDA]:
//! - `WorldControllerRenderTarget::renderFogOfWar` 0x42FC60 clears the texture to black and
//!   draws one blob for each explored part of the cell's 16 x 16 bitmap.
//! - `WorldControllerRenderTarget::updateFogOfWarVertexBuffer` 0x42FA50 draws one blob: a
//!   screen polygon centred at (x / 16, y / 16) of the texture, `size / 16` wide, with the
//!   texture coordinates 0.0625 to 1 on the map's fog texture (`Textures\MapFOWTexture.tga`,
//!   or the ini key "Map FOW Texture"), filtered bilinearly, opaque white vertex colours,
//!   and the blend source alpha over inverse source alpha. The texture is opaque, so a blob
//!   writes its colour over what is there.
//!
//! The copy follows the helper's picture: the pixel centres at +0.5, and the blob's texels as
//! the GPU gets them (the engine's converter into the device format, then a 16-bit format
//! widened as the helper widens it, or a DXT1 block decoded). A texture without a copy (made
//! in another way) keeps the engine's lock. A copy goes when its texture is freed or when
//! pixels are copied into it (`own_host`).
//!
//! `NIRENDER_MAP_FOG=0` leaves the engine's locks in place. `NIRENDER_FOG_CHECK=1` also reads
//! each locked fog texture from the GPU and counts the differences, for a test.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::collections::HashMap;

use crate::engine::get_image_converter;
use crate::gen::layouts::{NiDX8SourceTextureData as SD, NiPixelData as PD, NiPropertyState as PS, NiScreenPolygon as SP, NiSourceTexture as ST, NiTexture as T, NiTexturingProperty as TP, NiTexturingProperty_Map as TM};
use crate::mem::rd;
use crate::ports::{ni, textures};
use crate::sync::LockCell;

/// `worldController`, its `mapRenderTarget`, and the target's `screenPolygon`.
const WORLD_CONTROLLER: usize = 0x7C67DC;
const MAP_RENDER_TARGET: usize = crate::gen::layouts::WorldController::mapRenderTarget;
const SCREEN_POLYGON: usize = crate::gen::layouts::WorldControllerRenderTarget::screenPolygon;
/// The texture coordinates of the blob's first corner (`sub_42D020`); the other is 1.
const BLOB_UV_START: f32 = 0.0625;
/// D3DFMT_DXT1.
const DXT1: u32 = 0x3154_5844;

/// A copy of a fog texture: 4 bytes for each pixel, blue first, as the helper's reads.
struct Copy {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    /// The blobs since the clear (x, y, size), for `NIRENDER_FOG_DUMP`.
    blobs: Vec<[f32; 3]>,
}

/// The texels of the blob as the GPU samples them: 4 bytes each, blue first.
struct Blob {
    width: u32,
    height: u32,
    texels: Vec<u8>,
}

static COPIES: LockCell<Option<HashMap<u32, Copy>>> = LockCell::new(None);
static BLOB: LockCell<Option<Blob>> = LockCell::new(None);
/// The locks that this module gave: the `D3DLOCKED_RECT` and the memory of its pixels.
static GIVEN: LockCell<Option<HashMap<usize, (Box<[u32; 2]>, Vec<u8>)>>> = LockCell::new(None);
/// The blob's texels could not be read: the module leaves the locks to the engine.
static NO_BLOB: AtomicBool = AtomicBool::new(false);

static INSTALLED: AtomicBool = AtomicBool::new(false);
static RENDER_FOG_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static DRAW_BLOB_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static LOCK_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static UNLOCK_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

static REBUILDS: AtomicU32 = AtomicU32::new(0);
static BLOBS: AtomicU32 = AtomicU32::new(0);
static LOCKS: AtomicU32 = AtomicU32::new(0);
static ENGINE_LOCKS: AtomicU32 = AtomicU32::new(0);
static CHECKS: AtomicU32 = AtomicU32::new(0);
static CHECK_PIXELS_OFF: AtomicU32 = AtomicU32::new(0);
static CHECK_LARGEST: AtomicU32 = AtomicU32::new(0);
static CHECK_TESTS_OFF: AtomicU32 = AtomicU32::new(0);

fn check_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("NIRENDER_FOG_CHECK").is_ok_and(|value| value == "1"))
}

/// Claims MWSE's four overrides (the render of the fog, a blob of it, the lock and the unlock
/// of a render target; MWSE's own fog cache locks through them too). The plugin calls this
/// when it is the host of the render link.
pub unsafe fn install() {
    if INSTALLED.load(Ordering::Relaxed) {
        return;
    }
    if std::env::var("NIRENDER_MAP_FOG").is_ok_and(|value| value == "0") {
        crate::log!("map fog: NIRENDER_MAP_FOG=0, the engine reads the fog from the GPU");
        return;
    }
    use crate::mwse_native::{claim, claim_all, NATIVE_OVERRIDE_FOG_BLOB, NATIVE_OVERRIDE_FOG_RENDER, NATIVE_OVERRIDE_RENDER_TARGET_LOCK, NATIVE_OVERRIDE_RENDER_TARGET_UNLOCK};
    // The unlock first: it must know the locks of this module before any is given.
    let claims = claim_all(&[
        (NATIVE_OVERRIDE_RENDER_TARGET_UNLOCK, unlock_detour as *const () as usize),
        (NATIVE_OVERRIDE_FOG_RENDER, render_fog_detour as *const () as usize),
        (NATIVE_OVERRIDE_FOG_BLOB, draw_blob_detour as *const () as usize),
    ]);
    let claims = match claims {
        Ok(claims) => claims,
        Err(error) => {
            // A copy that misses a draw would give old pixels: the locks stay the engine's.
            crate::log!("map fog: {error}: the engine reads the fog from the GPU");
            return;
        }
    };
    UNLOCK_ORIGINAL.store(claims[0].engine, Ordering::Relaxed);
    RENDER_FOG_ORIGINAL.store(claims[1].engine, Ordering::Relaxed);
    DRAW_BLOB_ORIGINAL.store(claims[2].engine, Ordering::Relaxed);
    // The copies follow the draws from now on; the locks use them only after this.
    INSTALLED.store(true, Ordering::Relaxed);
    match claim(NATIVE_OVERRIDE_RENDER_TARGET_LOCK, lock_detour as *const () as usize) {
        Ok(lock) => LOCK_ORIGINAL.store(lock.engine, Ordering::Relaxed),
        Err(error) => {
            crate::log!("map fog: {error}: the engine reads the fog from the GPU");
            return;
        }
    }
    crate::log!("map fog: the plugin keeps the fog of war on the CPU; a lock of a fog texture reads the copy{}", if check_on() { " (NIRENDER_FOG_CHECK=1: each lock also reads the GPU)" } else { "" });
}

/// The device texture (the handle of the own host) of a rendered texture, or 0.
unsafe fn handle_of(rendered: usize) -> u32 {
    if rendered == 0 {
        return 0;
    }
    let data: usize = rd(rendered, T::rendererData);
    let as_rendered = if data != 0 { textures::data_get_as_rendered(data) } else { 0 };
    if as_rendered != 0 { textures::data_get_d3d_texture(as_rendered) as u32 } else { 0 }
}

/// A texture is freed, or pixels are copied into it: its copy is old.
pub fn forget(lock: &crate::sync::DeviceLock, handle: u32) {
    if let Some(copies) = COPIES.get(lock).as_mut() {
        copies.remove(&handle);
    }
}

/// `renderFogOfWar` (thiscall): the texture, the 16 x 16 bitmap, a flag for small blobs.
unsafe extern "thiscall" fn render_fog_detour(this: usize, texture: usize, bitmap: usize, flag: u32) {
    let handle = handle_of(texture);
    if handle != 0 {
        if let Some((width, height)) = crate::own_host::texture_size(handle) {
            let lock = crate::sync::device_lock();
            // The clear: black. The blobs come through `draw_blob_detour`.
            COPIES.get(&lock).get_or_insert_with(HashMap::new).insert(handle, Copy { width, height, pixels: vec![0; (width * height * 4) as usize], blobs: Vec::new() });
            REBUILDS.fetch_add(1, Ordering::Relaxed);
        }
    }
    let original: unsafe extern "thiscall" fn(usize, usize, usize, u32) = core::mem::transmute(RENDER_FOG_ORIGINAL.load(Ordering::Relaxed));
    original(this, texture, bitmap, flag);
}

/// `updateFogOfWarVertexBuffer` (thiscall): the texture (a counted pointer that the function
/// lets go of), x and y in sixteenths of the texture (y is a 16-bit number), the size in
/// sixteenths.
unsafe extern "thiscall" fn draw_blob_detour(this: usize, texture: usize, x: i32, y: i32, size: u32) {
    // The handle first: the function can let go of the last reference to the texture.
    let handle = handle_of(texture);
    let original: unsafe extern "thiscall" fn(usize, usize, i32, i32, u32) = core::mem::transmute(DRAW_BLOB_ORIGINAL.load(Ordering::Relaxed));
    original(this, texture, x, y, size);
    if handle == 0 || NO_BLOB.load(Ordering::Relaxed) {
        return;
    }
    let lock = crate::sync::device_lock();
    if BLOB.get(&lock).is_none() && !read_blob(&lock) {
        NO_BLOB.store(true, Ordering::Relaxed);
        crate::log!("map fog: the texels of the fog texture could not be read: the engine reads the fog from the GPU");
        if let Some(copies) = COPIES.get(&lock).as_mut() {
            copies.clear();
        }
        return;
    }
    let mut copies = COPIES.get(&lock);
    let Some(copy) = copies.as_mut().and_then(|copies| copies.get_mut(&handle)) else { return };
    let blob_open = BLOB.get(&lock);
    let blob = blob_open.as_ref().unwrap();
    draw_blob(copy, blob, x as i16 as f32, y as i16 as f32, f32::from_bits(size));
    if copy.blobs.len() < 4096 {
        copy.blobs.push([x as i16 as f32, y as i16 as f32, f32::from_bits(size)]);
    }
    BLOBS.fetch_add(1, Ordering::Relaxed);
}

/// Draws a blob into a copy as the helper draws the screen polygon: the corners as the engine
/// computes them, with the screen polygon's y from the bottom row of the texture (a test of
/// 2026-10-21 against the GPU, records entry 56); a pixel covered when its centre is in the
/// square (no centre is on an edge: the edges are at whole 64ths); the texture coordinates
/// interpolated and sampled bilinearly with wrapping; the sample added to the pixel and
/// clamped. The blend is one and one: `setupTarget` 0x42D4F0 sets source alpha over inverse
/// source alpha, then writes `flags & ~0x1FF | 1`, which clears both blend fields to 0 (one).
fn draw_blob(copy: &mut Copy, blob: &Blob, x: f32, y: f32, size: f32) {
    // The engine: the size in sixteenths, the half size, the corners (floats in its vertices).
    let side = size * 0.0625;
    let half = side * 0.5;
    let left = (x as f64 * 0.0625 - half as f64) as f32;
    let top = (y as f64 * 0.0625 - half as f64) as f32;
    let right = left + side;
    let bottom = top + side;
    let (w, h) = (copy.width as f32, copy.height as f32);
    for row in 0..copy.height {
        let cy = 1.0 - row as f32 / h;
        if cy < top || cy >= bottom {
            continue;
        }
        let v = BLOB_UV_START + (cy - top) / side * (1.0 - BLOB_UV_START);
        for column in 0..copy.width {
            let cx = column as f32 / w;
            if cx < left || cx >= right {
                continue;
            }
            let u = BLOB_UV_START + (cx - left) / side * (1.0 - BLOB_UV_START);
            let texel = sample(blob, u, v);
            let at = ((row * copy.width + column) * 4) as usize;
            for c in 0..4 {
                copy.pixels[at + c] = (copy.pixels[at + c] as f32 + texel[c]).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// A bilinear sample with wrapping, in levels of 255 before the rounding: blue, green, red,
/// alpha.
fn sample(blob: &Blob, u: f32, v: f32) -> [f32; 4] {
    let (tw, th) = (blob.width as i64, blob.height as i64);
    let fx = u * tw as f32 - 0.5;
    let fy = v * th as f32 - 0.5;
    let (x0, y0) = (fx.floor(), fy.floor());
    let (ax, ay) = (fx - x0, fy - y0);
    let texel = |x: i64, y: i64, c: usize| -> f32 {
        let (x, y) = (x.rem_euclid(tw), y.rem_euclid(th));
        blob.texels[((y * tw + x) * 4) as usize + c] as f32
    };
    let (x0, y0) = (x0 as i64, y0 as i64);
    let mut out = [0f32; 4];
    for (c, value) in out.iter_mut().enumerate() {
        let top = texel(x0, y0, c) * (1.0 - ax) + texel(x0 + 1, y0, c) * ax;
        let bottom = texel(x0, y0 + 1, c) * (1.0 - ax) + texel(x0 + 1, y0 + 1, c) * ax;
        *value = top * (1.0 - ay) + bottom * ay;
    }
    out
}

/// Reads the texels of the blob's texture as the GPU has them: the base map of the screen
/// polygon's texturing property, its file read by the engine's converter, converted into the
/// device format of the texture, then widened or decoded.
unsafe fn read_blob(lock: &crate::sync::DeviceLock) -> bool {
    let world: usize = rd(WORLD_CONTROLLER, 0);
    let polygon: usize = if world != 0 { rd(world + MAP_RENDER_TARGET, SCREEN_POLYGON) } else { 0 };
    let state: usize = if polygon != 0 { rd(polygon, SP::propertyState) } else { 0 };
    let texturing: usize = if state != 0 { rd(state, PS::propTexturing) } else { 0 };
    let maps: usize = if texturing != 0 { rd(texturing, TP::maps_storage) } else { 0 };
    let map: usize = if maps != 0 { rd(maps, 0) } else { 0 };
    let texture: usize = if map != 0 { rd(map, TM::texture) } else { 0 };
    let data: usize = if texture != 0 { rd(texture, T::rendererData) } else { 0 };
    if data == 0 {
        return false;
    }
    let d3d_format: u32 = rd(data, SD::format_d3dFormat);
    let converter = get_image_converter();
    let filename: usize = rd(texture, ST::filenameOnPC);
    let source: usize = if filename != 0 { textures::converter_read_file(converter, filename, 0) } else { rd(texture, ST::pixelData) };
    if source == 0 {
        return false;
    }
    ni::add_reference(source);
    let converted = textures::converter_convert(converter, source, data + SD::format, source, 0);
    let mut ok = false;
    if converted != 0 {
        let (width, height): (u32, u32) = (rd(rd::<usize>(converted, PD::mipmapWidths), 0), rd(rd::<usize>(converted, PD::mipmapHeights), 0));
        let bytes: u32 = rd(converted, PD::bytesPerPixel);
        let offsets: usize = rd(converted, PD::mipmapOffsets);
        let (start, end): (u32, u32) = (rd(offsets, 0), rd(offsets, 4));
        let pixels = (rd::<usize>(converted, PD::pixelsAllMips) + start as usize) as *const u8;
        let level = core::slice::from_raw_parts(pixels, end.saturating_sub(start) as usize);
        let texels: Option<Vec<u8>> = match (d3d_format, bytes) {
            (21, 4) => Some(level[..(width * height * 4) as usize].to_vec()),
            (22, 4) => Some(level[..(width * height * 4) as usize].chunks_exact(4).flat_map(|p| [p[0], p[1], p[2], 255]).collect()),
            (23..=26, 2) => Some(widen_16_bits(d3d_format, &level[..(width * height * 2) as usize])),
            (DXT1, _) if width % 4 == 0 && height % 4 == 0 && level.len() >= (width / 4 * height / 4 * 8) as usize => Some(decode_dxt1(width, height, level)),
            _ => None,
        };
        if let Ok(folder) = std::env::var("NIRENDER_FOG_DUMP") {
            let _ = std::fs::write(format!("{folder}/blob-level0-{d3d_format:#x}.bin"), level);
        }
        if let Some(texels) = texels {
            crate::log!("map fog: the fog texture is {width} x {height}, D3D format {d3d_format:#x}");
            *BLOB.get(lock) = Some(Blob { width, height, texels });
            ok = true;
        } else {
            crate::log!("map fog: the fog texture has the D3D format {d3d_format:#x} with {bytes} bytes a pixel, {width} x {height}, which this module does not read");
        }
        if converted != source && rd::<u32>(converted, ni::REF_COUNT) == 0 {
            ni::delete_object(converted);
        }
    }
    ni::release(source);
    ok
}

/// The helper's widening of a 16-bit format (`host/src/d3d.rs` `expand_16_bits`): blue first.
fn widen_16_bits(d3d_format: u32, source: &[u8]) -> Vec<u8> {
    let wide = |value: u16, bits: u32| -> u8 { ((value as u32 * 255 + ((1 << bits) - 1) / 2) / ((1 << bits) - 1)) as u8 };
    let mut pixels = Vec::with_capacity(source.len() * 2);
    for pair in source.chunks_exact(2) {
        let p = u16::from_le_bytes([pair[0], pair[1]]);
        let (red, green, blue, alpha) = match d3d_format {
            23 => (wide(p >> 11, 5), wide(p >> 5 & 63, 6), wide(p & 31, 5), 255),
            24 => (wide(p >> 10 & 31, 5), wide(p >> 5 & 31, 5), wide(p & 31, 5), 255),
            25 => (wide(p >> 10 & 31, 5), wide(p >> 5 & 31, 5), wide(p & 31, 5), if p >> 15 != 0 { 255 } else { 0 }),
            _ => (wide(p >> 8 & 15, 4), wide(p >> 4 & 15, 4), wide(p & 15, 4), wide(p >> 12, 4)),
        };
        pixels.extend_from_slice(&[blue, green, red, alpha]);
    }
    pixels
}

/// The texels of DXT1 blocks: the end colours widened by repeating their high bits, the two
/// colours between them at a third and two thirds (or the middle and a transparent black when
/// the first colour is not the greater), as the BC1 rule of Direct3D. Blue first.
fn decode_dxt1(width: u32, height: u32, blocks: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; (width * height * 4) as usize];
    let colour = |c: u16| -> [i32; 3] {
        let (r, g, b) = ((c >> 11) as i32, (c >> 5 & 63) as i32, (c & 31) as i32);
        [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)]
    };
    let mut at = 0;
    for by in 0..height / 4 {
        for bx in 0..width / 4 {
            let block = &blocks[at..at + 8];
            at += 8;
            let (c0, c1) = (u16::from_le_bytes([block[0], block[1]]), u16::from_le_bytes([block[2], block[3]]));
            let (a, b) = (colour(c0), colour(c1));
            let mut palette = [[0i32; 4]; 4];
            palette[0] = [a[0], a[1], a[2], 255];
            palette[1] = [b[0], b[1], b[2], 255];
            if c0 > c1 {
                for c in 0..3 {
                    palette[2][c] = (2 * a[c] + b[c] + 1) / 3;
                    palette[3][c] = (a[c] + 2 * b[c] + 1) / 3;
                }
                palette[2][3] = 255;
                palette[3][3] = 255;
            } else {
                for c in 0..3 {
                    palette[2][c] = (a[c] + b[c]) / 2;
                }
                palette[2][3] = 255;
                palette[3] = [0, 0, 0, 0];
            }
            let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
            for i in 0..16u32 {
                let p = palette[(indices >> (2 * i) & 3) as usize];
                let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                let o = ((y * width + x) * 4) as usize;
                out[o..o + 4].copy_from_slice(&[p[2] as u8, p[1] as u8, p[0] as u8, p[3] as u8]);
            }
        }
    }
    out
}

/// `lockRenderTarget` (thiscall: the texture, 0 for the target's own). Returns a
/// `D3DLOCKED_RECT`, or 0. The function lets go of one reference to a texture that is not 0.
/// A fog texture with a copy gets the copy's pixels.
unsafe extern "thiscall" fn lock_detour(this: usize, texture: usize) -> usize {
    let engine = |texture: usize| -> usize {
        let original: unsafe extern "thiscall" fn(usize, usize) -> usize = core::mem::transmute(LOCK_ORIGINAL.load(Ordering::Relaxed));
        original(this, texture)
    };
    let handle = handle_of(texture);
    if handle == 0 || NO_BLOB.load(Ordering::Relaxed) {
        ENGINE_LOCKS.fetch_add(1, Ordering::Relaxed);
        return engine(texture);
    }
    let lock = crate::sync::device_lock();
    let Some(pixels) = COPIES.get(&lock).as_ref().and_then(|copies| copies.get(&handle)).map(|copy| (copy.width, copy.pixels.clone())) else {
        ENGINE_LOCKS.fetch_add(1, Ordering::Relaxed);
        drop(lock);
        return engine(texture);
    };
    if check_on() {
        check(&lock, handle);
    }
    let (width, pixels) = pixels;
    let rect = Box::new([width * 4, pixels.as_ptr() as u32]);
    let address = &*rect as *const [u32; 2] as usize;
    GIVEN.get(&lock).get_or_insert_with(HashMap::new).insert(address, (rect, pixels));
    LOCKS.fetch_add(1, Ordering::Relaxed);
    drop(lock);
    // As the engine function: one reference let go.
    ni::release(texture);
    address
}

/// `unlockRenderTarget` (thiscall: the `D3DLOCKED_RECT`, a flag to copy back). A lock of this
/// module is let go here; the engine's goes to the engine.
unsafe extern "thiscall" fn unlock_detour(this: usize, rect: usize, copy_back: u32) {
    {
        let lock = crate::sync::device_lock();
        if GIVEN.get(&lock).as_mut().is_some_and(|given| given.remove(&rect).is_some()) {
            return;
        }
    }
    let original: unsafe extern "thiscall" fn(usize, usize, u32) = core::mem::transmute(UNLOCK_ORIGINAL.load(Ordering::Relaxed));
    original(this, rect, copy_back);
}

/// The test of `NIRENDER_FOG_CHECK=1`: the texture from the GPU against the copy, and the
/// tests of the map menu (green over 2 outside, over 32 inside) for each pixel.
unsafe fn check(lock: &crate::sync::DeviceLock, handle: u32) {
    let Some((width, height, pixels)) = crate::capture::read_pixels(handle) else { return };
    let copies = COPIES.get(lock);
    let Some(copy) = copies.as_ref().and_then(|copies| copies.get(&handle)) else { return };
    CHECKS.fetch_add(1, Ordering::Relaxed);
    if (width, height) != (copy.width, copy.height) {
        CHECK_PIXELS_OFF.fetch_add(width * height, Ordering::Relaxed);
        return;
    }
    let (mut off, mut largest, mut tests) = (0u32, 0u32, 0u32);
    for (gpu, cpu) in pixels.chunks_exact(4).zip(copy.pixels.chunks_exact(4)) {
        let difference = (0..3).map(|c| (gpu[c] as i32 - cpu[c] as i32).unsigned_abs()).max().unwrap_or(0);
        if difference > 0 {
            off += 1;
            largest = largest.max(difference);
        }
        if (gpu[1] > 2) != (cpu[1] > 2) || (gpu[1] > 32) != (cpu[1] > 32) {
            tests += 1;
        }
    }
    CHECK_PIXELS_OFF.fetch_add(off, Ordering::Relaxed);
    CHECK_LARGEST.fetch_max(largest, Ordering::Relaxed);
    CHECK_TESTS_OFF.fetch_add(tests, Ordering::Relaxed);
    // `NIRENDER_FOG_DUMP=<folder>`: the first 8 pairs that differ, as raw files of 4 bytes a
    // pixel (blue first), for a look.
    static DUMPED: AtomicU32 = AtomicU32::new(0);
    if off != 0 {
        if let Ok(folder) = std::env::var("NIRENDER_FOG_DUMP") {
            let n = DUMPED.fetch_add(1, Ordering::Relaxed);
            if n < 8 {
                let _ = std::fs::write(format!("{folder}/fog-{n}-{width}x{height}-gpu.raw"), pixels);
                let _ = std::fs::write(format!("{folder}/fog-{n}-{width}x{height}-cpu.raw"), &copy.pixels);
                let blobs: String = copy.blobs.iter().map(|b| format!("{} {} {}\n", b[0], b[1], b[2])).collect();
                let _ = std::fs::write(format!("{folder}/fog-{n}-blobs.txt"), blobs);
                if n == 0 {
                    if let Some(blob) = BLOB.get(lock).as_ref() {
                        let _ = std::fs::write(format!("{folder}/blob-{}x{}.raw", blob.width, blob.height), &blob.texels);
                    }
                }
            }
        }
    }
    static LOGGED: AtomicU32 = AtomicU32::new(0);
    if off != 0 && LOGGED.fetch_add(1, Ordering::Relaxed) < 10 {
        crate::log!("map fog check: the texture {handle:#x}: {off} of {} pixels differ, by {largest} at most; {tests} pixels give another test", width * height);
    }
}

/// The counts, for the log at the end of the stream.
pub fn report() -> String {
    if !INSTALLED.load(Ordering::Relaxed) {
        return "map fog: off; the engine reads the fog from the GPU".to_string();
    }
    let mut text = format!(
        "map fog: {} rebuilds and {} blobs drawn into the copies; {} locks from the copies, {} locks by the engine{}",
        REBUILDS.load(Ordering::Relaxed),
        BLOBS.load(Ordering::Relaxed),
        LOCKS.load(Ordering::Relaxed),
        ENGINE_LOCKS.load(Ordering::Relaxed),
        if NO_BLOB.load(Ordering::Relaxed) { " (the fog texture was not read)" } else { "" },
    );
    if check_on() {
        text.push_str(&format!(
            "; check: {} locks checked, {} pixels differ (by {} at most), {} pixels give another test of the map menu",
            CHECKS.load(Ordering::Relaxed),
            CHECK_PIXELS_OFF.load(Ordering::Relaxed),
            CHECK_LARGEST.load(Ordering::Relaxed),
            CHECK_TESTS_OFF.load(Ordering::Relaxed),
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grey_blob() -> Blob {
        // 16 x 16 with 255 in the middle texel (8, 8), 0 elsewhere.
        let mut texels = vec![0u8; 16 * 16 * 4];
        let at = (8 * 16 + 8) * 4;
        texels[at..at + 4].copy_from_slice(&[255, 255, 255, 255]);
        Blob { width: 16, height: 16, texels }
    }

    #[test]
    fn a_blob_adds_to_the_pixels_whose_positions_are_in_its_square() {
        let mut copy = Copy { width: 64, height: 64, pixels: vec![7; 64 * 64 * 4], blobs: Vec::new() };
        // Centre (8/16, 8/16), size 4/16 = 16 pixels. Columns at c/64 in [24/64, 40/64): 24 to
        // 39. Rows at 1 - r/64 in the same: r/64 in (24/64, 40/64]: 25 to 40.
        let mut blob = grey_blob();
        blob.texels.iter_mut().for_each(|t| *t = 10);
        draw_blob(&mut copy, &blob, 8.0, 8.0, 4.0);
        let written = |c: usize, r: usize| copy.pixels[(r * 64 + c) * 4 + 1] == 17;
        assert!(written(24, 25) && written(39, 40));
        assert!(!written(23, 30) && !written(40, 30) && !written(30, 24) && !written(30, 41));
    }

    #[test]
    fn the_screen_polygon_y_counts_from_the_bottom_row() {
        let mut copy = Copy { width: 64, height: 64, pixels: vec![0; 64 * 64 * 4], blobs: Vec::new() };
        let mut blob = grey_blob();
        blob.texels.iter_mut().for_each(|t| *t = 10);
        // A blob at the top of the polygon's space (y = 2/16) is near the last rows.
        draw_blob(&mut copy, &blob, 8.0, 2.0, 2.0);
        let green = |c: usize, r: usize| copy.pixels[(r * 64 + c) * 4 + 1];
        assert_eq!(green(32, 56), 10);
        assert_eq!(green(32, 8), 0);
    }

    #[test]
    fn the_16_bit_widening_is_the_helpers() {
        // R5G6B5 white and a green of 32 of 63.
        assert_eq!(widen_16_bits(23, &0xFFFFu16.to_le_bytes()), vec![255, 255, 255, 255]);
        assert_eq!(widen_16_bits(23, &(32u16 << 5).to_le_bytes())[1], 130);
    }

    #[test]
    fn a_dxt1_block_gives_its_end_colours_and_the_thirds() {
        // White and black, the indices 0, 1, 2, 3 in the first row.
        let block = [0xFF, 0xFF, 0x00, 0x00, 0b1110_0100, 0, 0, 0];
        let texels = decode_dxt1(4, 4, &block);
        let green = |i: usize| texels[i * 4 + 1];
        assert_eq!((green(0), green(1), green(2), green(3)), (255, 0, 170, 85));
    }
}
