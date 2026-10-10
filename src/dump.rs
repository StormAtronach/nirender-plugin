//! Dumps for the comparison of two runs of the game.
//!
//! The shadow test cannot compare the code that runs one time at the start of the game. That
//! code is the device creation and the constructors. For that code the tool compares two
//! whole runs. One run started with the game functions and the other started with the Rust
//! functions. At the same point of the two runs the plugin writes files. The files hold the
//! state of the renderer objects and the device calls of one frame.
//! `tools/compare_runs.py` compares the files.
//!
//! Addresses differ from run to run. So the tool does not compare the value of a word that
//! is an address. In the state dump the word gets a mark. In the frame dump the word becomes
//! a number. The number is the order in which the addresses first appear. Thus "the same
//! texture as in call 5" stays comparable.

use core::ffi::c_void;
use std::collections::HashMap;
use std::io::Write;

use crate::device;
use crate::engine;
use crate::gen::layouts::{NiDX8ConfigurableTexturePipeline, NiDX8LightManager, NiDX8RenderState, NiDX8Renderer};
use crate::mem::rd;

#[repr(C)]
struct MemoryBasicInformation {
    base_address: usize,
    allocation_base: usize,
    allocation_protect: u32,
    region_size: usize,
    state: u32,
    protect: u32,
    kind: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn VirtualQuery(address: *const c_void, buffer: *mut MemoryBasicInformation, length: usize) -> usize;
}

/// True when `value` is the address of committed memory outside the image of the game. The
/// image is at the same place in every run. Thus the tool compares its addresses as plain
/// numbers.
pub unsafe fn is_run_address(value: u32) -> bool {
    let value = value as usize;
    if value < 0x10000 || (0x400000..0x800000).contains(&value) {
        return false;
    }
    let mut info: MemoryBasicInformation = core::mem::zeroed();
    if VirtualQuery(value as *const c_void, &mut info, core::mem::size_of::<MemoryBasicInformation>()) == 0 {
        return false;
    }
    const MEM_COMMIT: u32 = 0x1000;
    const PAGE_NOACCESS: u32 = 0x01;
    info.state == MEM_COMMIT && info.protect & PAGE_NOACCESS == 0
}

unsafe fn write_object(out: &mut impl Write, name: &str, address: usize, size: usize) -> std::io::Result<()> {
    writeln!(out, "object {name} {size:#x}")?;
    if address == 0 {
        return writeln!(out, "  null");
    }
    let words = size / 4;
    for row in (0..words).step_by(8) {
        let mut line = format!("  {:04x}:", row * 4);
        for index in row..(row + 8).min(words) {
            let value: u32 = rd(address, index * 4);
            // An address keeps its value and gets a mark. The reader takes two marked words as
            // equal. The reader still compares a small number that only looks like an address.
            let mark = if is_run_address(value) { '*' } else { ' ' };
            line.push_str(&format!(" {mark}{value:08x}"));
        }
        writeln!(out, "{line}")?;
    }
    Ok(())
}

/// Writes the renderer and the objects it owns to `path`.
pub unsafe fn state(path: &str) -> std::io::Result<()> {
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    let renderer = engine::renderer();
    write_object(&mut out, "NiDX8Renderer", renderer, NiDX8Renderer::SIZE)?;
    if renderer == 0 {
        return Ok(());
    }
    let member = |offset: usize| -> usize { rd(renderer, offset) };
    write_object(&mut out, "NiDX8RenderState", member(NiDX8Renderer::renderState), NiDX8RenderState::SIZE)?;
    write_object(&mut out, "NiDX8VertexBufferManager", member(NiDX8Renderer::vertexBufferManager), 0x3C)?;
    write_object(&mut out, "NiDX8IndexBufferManager", member(NiDX8Renderer::indexBufferManager), 0x4C)?;
    write_object(&mut out, "NiDX8TextureManager", member(NiDX8Renderer::textureManager), 0x40)?;
    write_object(&mut out, "NiDX8LightManager", member(NiDX8Renderer::lightManager), NiDX8LightManager::SIZE)?;
    write_object(
        &mut out,
        "NiDX8ConfigurableTexturePipeline",
        member(NiDX8Renderer::configurableTexturePipeline),
        NiDX8ConfigurableTexturePipeline::SIZE,
    )?;
    out.flush()
}

/// Writes a frame log to `path`, one line per record, with run addresses as numbers of
/// first appearance.
pub unsafe fn frame(path: &str, log: &[u32]) -> std::io::Result<()> {
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut seen: HashMap<u32, u32> = HashMap::new();
    let name = |value: u32, seen: &mut HashMap<u32, u32>| -> String {
        if is_run_address(value) {
            let next = seen.len() as u32;
            format!("@{}", *seen.entry(value).or_insert(next))
        } else {
            format!("{value:x}")
        }
    };
    let mut at = 0;
    while let Some((record, next)) = device::read_record(log, at) {
        at = next;
        let method = device::slot_name(record.slot);
        let args: Vec<String> = record.args.iter().map(|&a| name(a, &mut seen)).collect();
        // The data of a call is numbers, such as matrices and vertices, and not addresses. So
        // the dump uses the data unchanged.
        let data = if record.payload.is_empty() {
            String::new()
        } else {
            format!(" data {} {:08x}", record.payload.len(), crate::mem::fnv1a(core::slice::from_raw_parts(record.payload.as_ptr() as *const u8, record.payload.len() * 4)))
        };
        writeln!(out, "{}({}){}", method, args.join(", "), data)?;
    }
    out.flush()
}
