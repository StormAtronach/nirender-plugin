//! The tiles of the local map, copied on the GPU (milestone 17, records entry 55).
//!
//! The engine draws each tile of the local map into one rendered texture
//! (`WorldControllerRenderTarget::drawMap` 0x42E320) and reads its pixels back at once
//! (`WorldControllerRenderTarget::renderMap` 0x42F6F0). `lockRenderTarget` copies the target
//! into an image surface, and `readbackRenderedTexture` 0x42F830 makes a `NiSourceTexture` of
//! the pixels. On the plugin's device the copy is a pixel read: the helper sends its frame to
//! the GPU and waits for it. A tile of 256 x 256 costs about 15 ms, and a cell cross draws
//! about 6 tiles (records entry 54).
//!
//! With this module the helper copies the target into the texture of the tile on the GPU, and
//! sends the pixels some frames later. The detour of `renderMap`:
//! 1. gives `readbackRenderedTexture` a memory of zeros in place of the locked surface. It
//!    makes the pixel data and the source texture as for the engine.
//! 2. makes the device texture of the source texture at once
//!    (`NiDX8SourceTextureData_static::Create` 0x6BFA50).
//! 3. asks the helper to put a copy of the target with alpha 1 into that texture, and to read
//!    the copy without a wait (`capture::copy_tile`).
//!
//! When the pixels come (`poll`, after each Present), they go into the pixel data as the
//! engine puts them there: bytes 0 and 2 swapped, alpha 255. The pixel data keeps its
//! revision, because the device texture has the pixels already.
//!
//! Other code reads the pixels of a tile later:
//! - `RecordsHandler::mapDrawCell` 0x4C81C0 paints the centre tile into the world map. Its
//!   call at 0x4E32FE waits in a list until the pixels of the tile came.
//! - `DataHandler::purgeTextures` 0x485490 makes the textures of the map again. Its detour
//!   completes all open tiles first, with a wait.
//! - A save writes the world map. The call of `createSaveScreenshot` at 0x4B586D, at the
//!   start of each save, completes all open tiles first, with a wait.
//! - `-createmaps` (`DataHandler::flagTestingCells`) keeps the engine's read.
//!
//! `NIRENDER_MAP_TILES=0` leaves the engine's read in place, for an A/B.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use crate::engine::{cdecl1, cdecl2, thiscall1};
use crate::gen::layouts::{NiPixelData as PD, NiSourceTexture as ST, NiTexture as T};
use crate::main_thread::{MainCell, MainThread};
use crate::mem::rd;
use crate::ports::{ni, textures};

/// `WorldControllerRenderTarget::renderMap` and the instructions that the jump displaces:
/// `mov al, [esp+4]`, `test al, al`.
const RENDER_MAP: usize = 0x42F6F0;
const RENDER_MAP_HEAD: [u8; 6] = [0x8A, 0x44, 0x24, 0x04, 0x84, 0xC0];
/// `WorldControllerRenderTarget::readbackRenderedTexture` (thiscall, a `D3DLOCKED_RECT`).
const READBACK: usize = 0x42F830;
/// `DataHandler::purgeTextures` and the instructions that the jump displaces: `push ebx`,
/// `push ebp`, `mov ebp, ecx`, `mov ecx, [worldController]`.
const PURGE_TEXTURES: usize = 0x485490;
const PURGE_TEXTURES_HEAD: [u8; 10] = [0x53, 0x55, 0x8B, 0xE9, 0x8B, 0x0D, 0xDC, 0x67, 0x7C, 0x00];
/// The call of `RecordsHandler::mapDrawCell` in `Cell::sub_4E3210`, which puts a tile of the
/// cache on the map.
const PAINT_SITE: usize = 0x4E32FE;
const MAP_DRAW_CELL: usize = 0x4C81C0;
/// The call of `TES3File_static::createSaveScreenshot` (cdecl, the file name) in
/// `TES3File::writeSaveHeader`.
const SAVE_SHOT_SITE: usize = 0x4B586D;
const CREATE_SAVE_SCREENSHOT: usize = 0x4B4F80;
/// `NiDX8SourceTextureData_static::Create` (cdecl, the texture, the renderer).
const SOURCE_DATA_CREATE: usize = 0x6BFA50;
/// `dataHandler` and `DataHandler::flagTestingCells`.
const DATA_HANDLER: usize = 0x7C67E0;
const TESTING_CELLS: usize = 0xB4DD;
/// `WorldControllerRenderTarget`: the rendered texture, the copied texture, the size.
const RENDERED_TEXTURE: usize = 0x2C;
const COPIED_TEXTURE: usize = 0x30;
const TARGET_WIDTH: usize = 0x40;
const TARGET_HEIGHT: usize = 0x44;
/// `Cell::mappingVisuals` and `CellMappingVisuals::textureSource`.
const CELL_MAPPING_VISUALS: usize = 0x84;
const VISUALS_TEXTURE_SOURCE: usize = 0x14;

/// A tile whose pixels did not come yet. The module holds a reference to its pixel data.
struct Open {
    ticket: u32,
    pixel_data: usize,
    width: u32,
    height: u32,
    /// The count of `poll` calls when the tile opened.
    opened: u32,
}

/// The calls of `poll`: one after each Present.
static POLLS: AtomicU32 = AtomicU32::new(0);

/// `NIRENDER_MAP_TILES_HOLD=N`, a test switch: `poll` completes a tile only N Presents after it
/// opened, so that a save or a purge soon after a cell cross finds open tiles. 0 by default.
fn hold() -> u32 {
    static HOLD: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *HOLD.get_or_init(|| std::env::var("NIRENDER_MAP_TILES_HOLD").ok().and_then(|value| value.parse().ok()).unwrap_or(0))
}

static OPEN: MainCell<Vec<Open>> = MainCell::new(Vec::new());
/// The paints of the world map that wait for the pixels of their tile: the records handler
/// and the cell.
static PAINTS: MainCell<Vec<(usize, usize)>> = MainCell::new(Vec::new());
/// The memory of zeros that `readbackRenderedTexture` copies from.
static ZEROS: MainCell<Vec<u8>> = MainCell::new(Vec::new());
static LAST_TICKET: AtomicU32 = AtomicU32::new(0);

static INSTALLED: AtomicBool = AtomicBool::new(false);
static RENDER_MAP_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static PURGE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

static COPIED: AtomicU32 = AtomicU32::new(0);
static COMPLETED: AtomicU32 = AtomicU32::new(0);
static WAITED: AtomicU32 = AtomicU32::new(0);
static WITHOUT_PIXELS: AtomicU32 = AtomicU32::new(0);
static READ_AT_ONCE: AtomicU32 = AtomicU32::new(0);
static PAINTS_DEFERRED: AtomicU32 = AtomicU32::new(0);
static ENGINE_READS: AtomicU32 = AtomicU32::new(0);
/// The longest wait for open tiles before a purge or a save, in ticks.
static LONGEST_WAIT: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(0);
/// The waits for a free slot of the tile memory: the count, the time and the longest, in
/// ticks. The most tiles open at one time.
static ROOM_WAITS: AtomicU32 = AtomicU32::new(0);
static ROOM_WAIT_TICKS: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(0);
static ROOM_WAIT_LONGEST: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(0);
static MOST_OPEN: AtomicUsize = AtomicUsize::new(0);

/// The bytes of a `call` at `site` to `target`.
fn call_bytes(site: usize, target: usize) -> [u8; 5] {
    let mut bytes = [0xE8, 0, 0, 0, 0];
    bytes[1..].copy_from_slice(&(target.wrapping_sub(site + 5) as u32).to_le_bytes());
    bytes
}

/// True when the 5 bytes at `site` are a call of `target`.
unsafe fn calls(site: usize, target: usize) -> bool {
    core::slice::from_raw_parts(site as *const u8, 5) == call_bytes(site, target)
}

/// Puts the detours in place. The plugin calls this when it is the host of the render link.
/// All four places must hold the engine's code: without the deferred paint the world map would
/// get the zeros of an open tile.
pub unsafe fn install() {
    if INSTALLED.load(Ordering::Relaxed) {
        return;
    }
    if std::env::var("NIRENDER_MAP_TILES").is_ok_and(|value| value == "0") {
        crate::log!("map tiles: NIRENDER_MAP_TILES=0, the engine reads each tile");
        return;
    }
    let live = |at: usize, len: usize| core::slice::from_raw_parts(at as *const u8, len);
    let mut wrong = Vec::new();
    if live(RENDER_MAP, RENDER_MAP_HEAD.len()) != RENDER_MAP_HEAD {
        wrong.push(format!("renderMap {:02x?}", live(RENDER_MAP, RENDER_MAP_HEAD.len())));
    }
    if live(PURGE_TEXTURES, PURGE_TEXTURES_HEAD.len()) != PURGE_TEXTURES_HEAD {
        wrong.push(format!("purgeTextures {:02x?}", live(PURGE_TEXTURES, PURGE_TEXTURES_HEAD.len())));
    }
    if !calls(PAINT_SITE, MAP_DRAW_CELL) {
        wrong.push(format!("the paint call at {PAINT_SITE:#x} {:02x?}", live(PAINT_SITE, 5)));
    }
    if !calls(SAVE_SHOT_SITE, CREATE_SAVE_SCREENSHOT) {
        wrong.push(format!("the save picture call at {SAVE_SHOT_SITE:#x} {:02x?}", live(SAVE_SHOT_SITE, 5)));
    }
    if !wrong.is_empty() {
        crate::log!("map tiles: another mod changed the code ({}): the engine reads each tile", wrong.join("; "));
        return;
    }
    let render_map = match crate::hook::install_raw(RENDER_MAP, &RENDER_MAP_HEAD, &[], render_map_detour as *const () as usize, 1, crate::hook::Cleanup::Callee) {
        Ok(original) => original,
        Err(error) => {
            crate::log!("map tiles: no detour on renderMap ({error}): the engine reads each tile");
            return;
        }
    };
    RENDER_MAP_ORIGINAL.store(render_map, Ordering::Relaxed);
    match crate::hook::install_raw(PURGE_TEXTURES, &PURGE_TEXTURES_HEAD, &[], purge_detour as *const () as usize, 0, crate::hook::Cleanup::Callee) {
        Ok(original) => PURGE_ORIGINAL.store(original, Ordering::Relaxed),
        Err(error) => {
            // The detour of renderMap is in place. It must not copy: a purge would make the
            // textures of open tiles from zeros.
            crate::log!("map tiles: no detour on purgeTextures ({error}): the engine reads each tile");
            return;
        }
    }
    if !crate::win::patch(PAINT_SITE, &call_bytes(PAINT_SITE, paint_detour as *const () as usize))
        || !crate::win::patch(SAVE_SHOT_SITE, &call_bytes(SAVE_SHOT_SITE, save_shot_detour as *const () as usize))
    {
        crate::log!("map tiles: a call could not be written: the engine reads each tile");
        return;
    }
    INSTALLED.store(true, Ordering::Relaxed);
    crate::log!("map tiles: the helper copies each tile of the local map on the GPU; the pixels come later");
    if hold() != 0 {
        crate::log!("map tiles: NIRENDER_MAP_TILES_HOLD={}: a tile stays open {} Presents at least", hold(), hold());
    }
}

unsafe fn engine_render_map(this: usize, flag: u32) {
    let original: unsafe extern "thiscall" fn(usize, u32) = core::mem::transmute(RENDER_MAP_ORIGINAL.load(Ordering::Relaxed));
    original(this, flag);
}

/// `WorldControllerRenderTarget::renderMap` (thiscall, a bool). With the bool set the engine
/// does nothing.
unsafe extern "thiscall" fn render_map_detour(this: usize, flag: u32) {
    if flag as u8 != 0 {
        return;
    }
    let copied = match MainThread::here() {
        Some(t) if INSTALLED.load(Ordering::Relaxed) => copy(&t, this),
        _ => false,
    };
    if !copied {
        ENGINE_READS.fetch_add(1, Ordering::Relaxed);
        engine_render_map(this, flag);
    }
}

fn testing_cells() -> bool {
    unsafe {
        let data_handler: usize = rd(DATA_HANDLER, 0);
        data_handler != 0 && rd::<u8>(data_handler, TESTING_CELLS) != 0
    }
}

/// Makes the tile of the target of `this` without a read. Returns false when the engine must
/// read it.
unsafe fn copy(t: &MainThread, this: usize) -> bool {
    if testing_cells() || !crate::capture::recording() {
        return false;
    }
    let (width, height) = (rd::<u32>(this, TARGET_WIDTH), rd::<u32>(this, TARGET_HEIGHT));
    let size = width as usize * height as usize * 4;
    if size == 0 || size > crate::capture::TILE_BYTES {
        return false;
    }
    let rendered: usize = rd(this, RENDERED_TEXTURE);
    let data: usize = if rendered != 0 { rd(rendered, T::rendererData) } else { 0 };
    let as_rendered = if data != 0 { textures::data_get_as_rendered(data) } else { 0 };
    let target = if as_rendered != 0 { textures::data_get_d3d_texture(as_rendered) } else { 0 };
    if target == 0 {
        return false;
    }
    make_room(t);
    // The source texture and its pixel data, as the engine makes them from the locked surface.
    let zeros = ZEROS.get(t);
    if zeros.len() < size {
        zeros.resize(size, 0);
    }
    let locked: [u32; 2] = [width * 4, zeros.as_ptr() as u32];
    thiscall1(READBACK, this, locked.as_ptr() as u32);
    let texture: usize = rd(this, COPIED_TEXTURE);
    let pixel_data: usize = if texture != 0 { rd(texture, ST::pixelData) } else { 0 };
    if pixel_data == 0 {
        return true;
    }
    // The device texture now: the copy goes into it.
    if rd::<usize>(texture, T::rendererData) == 0 {
        cdecl2(SOURCE_DATA_CREATE, texture as u32, crate::engine::renderer() as u32);
    }
    let source_data: usize = rd(texture, T::rendererData);
    let handle = if source_data != 0 { textures::data_get_d3d_texture(source_data) } else { 0 };
    let ticket = if handle != 0 { crate::capture::copy_tile(target as u32, handle as u32) } else { None };
    let Some(ticket) = ticket else {
        // No copy: the pixels come from a read now, as for the engine.
        READ_AT_ONCE.fetch_add(1, Ordering::Relaxed);
        match crate::capture::read_pixels(target as u32) {
            Some((w, h, pixels)) if (w, h) == (width, height) => {
                fill(pixel_data, width, height, pixels);
            }
            _ => {
                WITHOUT_PIXELS.fetch_add(1, Ordering::Relaxed);
            }
        }
        return true;
    };
    LAST_TICKET.store(ticket, Ordering::Relaxed);
    ni::add_reference(pixel_data);
    OPEN.get(t).push(Open { ticket, pixel_data, width, height, opened: POLLS.load(Ordering::Relaxed) });
    MOST_OPEN.fetch_max(OPEN.get(t).len(), Ordering::Relaxed);
    COPIED.fetch_add(1, Ordering::Relaxed);
    true
}

/// Completes the oldest open tiles until the next ticket has a free slot in the tile memory.
unsafe fn make_room(t: &MainThread) {
    let next = LAST_TICKET.load(Ordering::Relaxed).wrapping_add(1);
    loop {
        let open = OPEN.get(t);
        let Some(oldest) = open.first() else { return };
        if (next.wrapping_sub(oldest.ticket) as usize) < crate::capture::TILE_SLOTS {
            return;
        }
        let oldest = open.remove(0);
        let start = crate::capture::ticks();
        let pixels = crate::capture::wait_tile(oldest.ticket);
        let waited = crate::capture::ticks() - start;
        ROOM_WAITS.fetch_add(1, Ordering::Relaxed);
        ROOM_WAIT_TICKS.fetch_add(waited, Ordering::Relaxed);
        ROOM_WAIT_LONGEST.fetch_max(waited, Ordering::Relaxed);
        complete(oldest, pixels);
    }
}

/// Puts pixels of the helper (4 bytes each, blue first) into the pixel data of a tile, as
/// `readbackRenderedTexture` puts the pixels of the surface there: bytes 0 and 2 swapped,
/// and alpha 255 as `renderMap` makes it.
unsafe fn fill(pixel_data: usize, width: u32, height: u32, pixels: &[u8]) -> bool {
    let size = width as usize * height as usize * 4;
    let widths: usize = rd(pixel_data, PD::mipmapWidths);
    let heights: usize = rd(pixel_data, PD::mipmapHeights);
    if pixels.len() < size || rd::<u32>(widths, 0) != width || rd::<u32>(heights, 0) != height || rd::<u32>(pixel_data, PD::bytesPerPixel) != 4 {
        return false;
    }
    let offsets: usize = rd(pixel_data, PD::mipmapOffsets);
    let to = (rd::<usize>(pixel_data, PD::pixelsAllMips) + rd::<u32>(offsets, 0) as usize) as *mut u8;
    let to = core::slice::from_raw_parts_mut(to, size);
    for (to, from) in to.chunks_exact_mut(4).zip(pixels.chunks_exact(4)) {
        to[0] = from[2];
        to[1] = from[1];
        to[2] = from[0];
        to[3] = 255;
    }
    true
}

/// The end of an open tile: its pixels, when they came, go into its pixel data.
unsafe fn complete(open: Open, pixels: Option<(u32, u32, &[u8])>) {
    let filled = match pixels {
        Some((width, height, pixels)) if (width, height) == (open.width, open.height) => fill(open.pixel_data, width, height, pixels),
        _ => false,
    };
    if !filled {
        WITHOUT_PIXELS.fetch_add(1, Ordering::Relaxed);
    }
    COMPLETED.fetch_add(1, Ordering::Relaxed);
    ni::release(open.pixel_data);
}

/// The pixel data of the tile of a cell in the cache, or 0.
unsafe fn cell_pixel_data(cell: usize) -> usize {
    let visuals: usize = rd(cell, CELL_MAPPING_VISUALS);
    let texture: usize = if visuals != 0 { rd(visuals, VISUALS_TEXTURE_SOURCE) } else { 0 };
    if texture != 0 { rd(texture, ST::pixelData) } else { 0 }
}

fn is_open(t: &MainThread, pixel_data: usize) -> bool {
    pixel_data != 0 && OPEN.get(t).iter().any(|open| open.pixel_data == pixel_data)
}

/// The call of `RecordsHandler::mapDrawCell` at 0x4E32FE (thiscall, the cell). The paint
/// waits while the tile of the cell is open.
unsafe extern "thiscall" fn paint_detour(records: usize, cell: usize) {
    if let Some(t) = MainThread::here() {
        if is_open(&t, cell_pixel_data(cell)) {
            let paints = PAINTS.get(&t);
            if !paints.contains(&(records, cell)) {
                paints.push((records, cell));
            }
            PAINTS_DEFERRED.fetch_add(1, Ordering::Relaxed);
            return;
        }
    }
    thiscall1(MAP_DRAW_CELL, records, cell as u32);
}

/// Runs the paints whose tiles are complete.
unsafe fn run_paints(t: &MainThread) {
    if PAINTS.get(t).is_empty() {
        return;
    }
    let paints = core::mem::take(PAINTS.get(t));
    for (records, cell) in paints {
        if is_open(t, cell_pixel_data(cell)) {
            PAINTS.get(t).push((records, cell));
        } else {
            thiscall1(MAP_DRAW_CELL, records, cell as u32);
        }
    }
}

/// After each Present, on the main thread: the tiles whose pixels came, and their paints.
pub unsafe fn poll() {
    if !INSTALLED.load(Ordering::Relaxed) {
        return;
    }
    let Some(t) = MainThread::here() else { return };
    let polls = POLLS.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    if OPEN.get(&t).is_empty() && PAINTS.get(&t).is_empty() {
        return;
    }
    if !crate::capture::recording() {
        // The stream ended: no pixels come any more.
        for open in core::mem::take(OPEN.get(&t)) {
            complete(open, None);
        }
    }
    let mut index = 0;
    while index < OPEN.get(&t).len() {
        let (ticket, opened) = (OPEN.get(&t)[index].ticket, OPEN.get(&t)[index].opened);
        if polls.wrapping_sub(opened) < hold() {
            index += 1;
            continue;
        }
        match crate::capture::tile_pixels(ticket) {
            Some(pixels) => {
                let open = OPEN.get(&t).remove(index);
                complete(open, Some(pixels));
            }
            None => index += 1,
        }
    }
    run_paints(&t);
}

/// Completes every open tile, with a wait, and runs the paints.
unsafe fn complete_all(t: &MainThread, before: &str) {
    static LOGGED: AtomicU32 = AtomicU32::new(0);
    if LOGGED.fetch_add(1, Ordering::Relaxed) < 40 {
        crate::log!("map tiles: before {before}: {} tiles open, {} paints wait", OPEN.get(t).len(), PAINTS.get(t).len());
    }
    if OPEN.get(t).is_empty() && PAINTS.get(t).is_empty() {
        return;
    }
    let start = crate::capture::ticks();
    for open in core::mem::take(OPEN.get(t)) {
        let pixels = crate::capture::wait_tile(open.ticket);
        WAITED.fetch_add(1, Ordering::Relaxed);
        complete(open, pixels);
    }
    LONGEST_WAIT.fetch_max(crate::capture::ticks() - start, Ordering::Relaxed);
    run_paints(t);
}

/// `DataHandler::purgeTextures` (thiscall).
unsafe extern "thiscall" fn purge_detour(this: usize) {
    match MainThread::here() {
        Some(t) => complete_all(&t, "a purge"),
        None => {
            static LOGGED: AtomicBool = AtomicBool::new(false);
            if !LOGGED.swap(true, Ordering::Relaxed) {
                crate::log!("map tiles: a purge on another thread: the open tiles are not completed first");
            }
        }
    }
    let original: unsafe extern "thiscall" fn(usize) = core::mem::transmute(PURGE_ORIGINAL.load(Ordering::Relaxed));
    original(this);
}

/// The call of `TES3File_static::createSaveScreenshot` at the start of a save (cdecl, the
/// file name; returns the pixel data of the picture).
unsafe extern "C" fn save_shot_detour(filename: u32) -> u32 {
    if let Some(t) = MainThread::here() {
        complete_all(&t, "a save");
    }
    cdecl1(CREATE_SAVE_SCREENSHOT, filename)
}

/// The counts, for the log at the end of the stream.
pub fn report() -> String {
    if !INSTALLED.load(Ordering::Relaxed) {
        return "map tiles: the copy is off; the engine reads each tile".to_string();
    }
    let ms = |ticks: i64| ticks as f64 * 1000.0 / crate::capture::ticks_per_second() as f64;
    format!(
        "map tiles: {} copied on the GPU, {} completed ({} with a wait before a purge or a save, the longest {:.1} ms; {} with a wait for a free slot, {:.1} ms in all, the longest {:.1} ms; at most {} open; {} without pixels), {} read at once, {} read by the engine; {} paints of the world map waited for their tile",
        COPIED.load(Ordering::Relaxed),
        COMPLETED.load(Ordering::Relaxed),
        WAITED.load(Ordering::Relaxed),
        ms(LONGEST_WAIT.load(Ordering::Relaxed)),
        ROOM_WAITS.load(Ordering::Relaxed),
        ms(ROOM_WAIT_TICKS.load(Ordering::Relaxed)),
        ms(ROOM_WAIT_LONGEST.load(Ordering::Relaxed)),
        MOST_OPEN.load(Ordering::Relaxed),
        WITHOUT_PIXELS.load(Ordering::Relaxed),
        READ_AT_ONCE.load(Ordering::Relaxed),
        ENGINE_READS.load(Ordering::Relaxed),
        PAINTS_DEFERRED.load(Ordering::Relaxed),
    )
}
