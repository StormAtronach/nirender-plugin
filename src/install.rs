//! The installation of the plugin: the census, the detours, the recorder, and the report.

use core::sync::atomic::Ordering;

use crate::gen::funcs::FUNCS;
use crate::gen::layouts::NiDX8Renderer;
use crate::shadow::{self, Mode};
use crate::{census, device, engine, frame, hook, ports, site, x87};

/// `NiDX8Renderer::BeginPaint`. The game calls it before the first device call of a frame.
const BEGIN_PAINT: usize = 0x6AC520;

/// Runs after each call of `BeginPaint` until the recorder is on the device.
///
/// The recorder does not go on the device when the game creates it. A wrapper on
/// `IDirect3D8::CreateDevice` did that before, and MGE XE then did not install its own hooks.
unsafe fn after_begin_paint() {
    if matches!(attach_device(), Ok(Some(_))) {
        if let Some(port) = ports::all().find(|p| p.addr == BEGIN_PAINT) {
            port.after.store(0, Ordering::Relaxed);
        }
    }
}

/// True when a port follows the change that the census found.
unsafe fn port_follows(change: &census::Change) -> bool {
    let func = change.func;
    let live = core::slice::from_raw_parts(func.ea as *const u8, func.size);
    if let site::Kind::RedirectedCall(at) = site::classify(func, live, change.offset, change.len) {
        return ports::serves_site(at);
    }
    let expected = census::expected(func);
    (change.offset..change.offset + change.len)
        .filter(|&i| live[i] != expected[i])
        .all(|i| ports::follows(func.ea + i, |addr| *(addr as *const u8)))
}

/// Does the census and writes it to the log. It locks the port of a function that another
/// mod changed in a way that the port does not follow. With `hooked`, the entries hold the
/// jumps of this plugin. These jumps are not a change. Returns the number of changed
/// functions.
pub unsafe fn census(hooked: bool) -> usize {
    let mut changed: Vec<usize> = Vec::new();
    for change in &census::changes() {
        let port = ports::all().find(|p| p.addr == change.func.ea);
        if hooked && change.offset < 16 && port.map_or(false, |p| p.installed()) {
            continue;
        }
        changed.push(change.func.ea);
        let note = match port {
            None => "",
            Some(_) if port_follows(change) => " (the port follows the change)",
            Some(port) => {
                let ran = !port.locked() && port.mode() != Mode::Off;
                port.lock();
                if ran {
                    " (PORT LOCKED NOW; it ran as Rust before)"
                } else {
                    " (PORT LOCKED: it stays game code here)"
                }
            }
        };
        crate::log!("census: {}{}", census::describe(change), note);
    }
    changed.dedup();
    crate::log!("census: {} of {} functions differ from the database", changed.len(), FUNCS.len());
    changed.len()
}

/// Puts the detours on the engine functions and the recorder on the device. Every port
/// starts in mode off. A second call only puts the recorder on the device.
pub unsafe fn install() -> Result<String, String> {
    if ports::all().any(|p| p.installed()) {
        return Ok(attach_device()?.unwrap_or_else(|| "the game has no renderer yet".to_string()));
    }
    shadow::set_main_thread();
    crate::log!("x87 control word {:#06x}", x87::control_word());

    // The census runs before the detours, because the detours change the entry bytes.
    let changed = census(false);

    match crate::alloc::install() {
        Ok(()) => crate::log!("allocator hooks on: calls that allocate are compared"),
        Err(error) => crate::log!("no allocator hooks ({error}): calls that allocate are not compared"),
    }
    device::set_frame_hook(frame::on_present);

    let mut failed = 0;
    for port in ports::all() {
        match hook::install(port.addr, (port.detour)(), port.stack_args, port.cleanup) {
            Ok(trampoline) => port.orig.store(trampoline, Ordering::Relaxed),
            Err(error) => {
                failed += 1;
                crate::log!("no detour on {}: {}", port.name, error);
            }
        }
    }
    let attached = match attach_device()? {
        Some(text) => text,
        None => {
            // The game has no renderer yet. The recorder goes on the device at the first frame.
            if let Some(port) = ports::all().find(|p| p.addr == BEGIN_PAINT) {
                port.after.store(after_begin_paint as *const () as usize, Ordering::Relaxed);
            }
            "the game has no renderer yet".to_string()
        }
    };
    Ok(format!(
        "{} ports installed, {} failed, {} functions differ from the database; {}",
        ports::all().count() - failed,
        failed,
        changed,
        attached
    ))
}

/// Puts the recorder on the device of the renderer. Returns `None` when the game has no
/// renderer or no device yet.
pub unsafe fn attach_device() -> Result<Option<String>, String> {
    let renderer = engine::renderer();
    if renderer == 0 {
        return Ok(None);
    }
    let device_ptr: usize = crate::mem::rd(renderer, NiDX8Renderer::d3dDevice);
    if device_ptr == 0 {
        return Ok(None);
    }
    device::attach(device_ptr)?;
    Ok(Some(format!("renderer {renderer:#x}, device {device_ptr:#x} with the recorder")))
}

/// One line for each port. A line has the name, the address, the mode and the counts of the
/// port. Tabs separate the columns. The log and the tools use this text.
pub fn report() -> String {
    let mut text = String::from("port\taddr\tinstalled\tmode\tshadow_calls\tmismatches\tskipped\tlive_calls\tlocked\trust_runs\n");
    for port in ports::all() {
        text.push_str(&format!(
            "{}\t{:#x}\t{}\t{:?}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            port.name,
            port.addr,
            port.installed(),
            port.mode(),
            port.shadow_calls.get(),
            port.mismatches.get(),
            port.skipped.get(),
            port.live_calls.get(),
            port.locked(),
            port.rust_runs.get(),
        ));
    }
    text
}
