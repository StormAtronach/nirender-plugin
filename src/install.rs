//! The installation of the plugin: the census, the detours, the recorder, and the report.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::gen::funcs::FUNCS;
use crate::gen::layouts::NiDX8Renderer;
use crate::shadow::{self, Mode};
use crate::{census, device, engine, frame, hook, ports, site, x87};

/// `NiDX8Renderer::BeginPaint`. The game calls it before the first device call of a frame.
const BEGIN_PAINT: usize = 0x6AC520;

/// Runs before each call of `BeginPaint` until the recorder is on the device. `args[0]` is
/// the renderer: the world controller does not have it yet at the first frames.
///
/// The recorder must be there before the first frame. Without it, the Rust run of a
/// comparison makes its device calls on the device of the game: the scene opens two times,
/// the second `BeginScene` fails, and the game draws nothing more.
///
/// The recorder does not go on the device when the game creates it. A wrapper on
/// `IDirect3D8::CreateDevice` did that before, and MGE XE then did not install its own hooks.
unsafe fn before_begin_paint(args: &[usize]) {
    let device_ptr: usize = crate::mem::rd(args[0], NiDX8Renderer::d3dDevice);
    if device_ptr != 0 && device::attach(device_ptr).is_ok() {
        ATTACHED.store(true, Ordering::Relaxed);
        crate::log!("first frame: renderer {:#x}, device {device_ptr:#x} with the recorder", args[0]);
        if let Some(port) = ports::all().find(|p| p.addr == BEGIN_PAINT) {
            port.before.store(0, Ordering::Relaxed);
        }
    }
}

/// True when a port follows the change that the census found.
unsafe fn port_follows(change: &census::Change) -> bool {
    let func = change.func;
    let live = core::slice::from_raw_parts(func.ea as *const u8, func.size);
    follows_change(func, live, change.offset, change.len, |addr| *(addr as *const u8))
}

/// True when a port follows the run of `len` changed bytes at `offset` of `func`. `live` is
/// the function in memory, and `read` gives the byte in memory at an address.
fn follows_change(func: &crate::gen::funcs::Func, live: &[u8], offset: usize, len: usize, read: impl Fn(usize) -> u8) -> bool {
    if let site::Kind::RedirectedCall(at) = site::classify(func, live, offset, len) {
        return ports::serves_site(at);
    }
    let expected = census::expected(func);
    (offset..offset + len).filter(|&i| live[i] != expected[i]).all(|i| ports::follows(func.ea + i, &read))
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
                let feature = ports::code_patch_feature(change.func.ea + change.offset, change.len);
                if let Some(feature) = feature {
                    let mut options = LOCKED_BY_OPTIONS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                    if !options.contains(&feature) {
                        options.push(feature);
                    }
                    crate::log!(
                        "census: the change is in a record of the Morrowind Code Patch option \"{}\" (feature {feature})",
                        ports::code_patch_option(feature)
                    );
                }
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

/// Set when the plugin is the Direct3D 8 of the game and a port cannot run as Rust: the game
/// must not get the device of the plugin, and MGE XE goes on with Direct3D 9.
static REFUSED: AtomicBool = AtomicBool::new(false);

/// The features of the Morrowind Code Patch whose records hold a change that locked a port.
static LOCKED_BY_OPTIONS: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());

/// The features of the Code Patch that locked a port: the cause of a refusal that the player
/// can change.
pub fn locked_by_options() -> Vec<u32> {
    LOCKED_BY_OPTIONS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
}

/// True when the renderer is refused for this session.
pub fn refused() -> bool {
    REFUSED.load(Ordering::Relaxed)
}

const REFUSED_MESSAGE: &str = "the renderer is refused for this session (see the census above); the game keeps MGE XE on Direct3D 9";

/// The message of a refusal, with the options of the Code Patch that caused it.
fn refused_message() -> String {
    let options: Vec<String> =
        locked_by_options().iter().map(|&feature| format!("\"{}\" (feature {feature})", ports::code_patch_option(feature))).collect();
    if options.is_empty() {
        REFUSED_MESSAGE.to_string()
    } else {
        format!("{REFUSED_MESSAGE}; the cause is the Morrowind Code Patch option {}", options.join(", "))
    }
}

/// Refuses the renderer for the session when the game asks for its Direct3D 8 before a port
/// is installed: the Lua side of the plugin did not run (the Lua of MWSE failed, or
/// `mods/nirender/main.lua` is missing). Without the ports, the game would draw on a device
/// that nothing feeds, and it crashes. True when the renderer is refused.
pub fn refuse_without_ports() -> bool {
    if refused() {
        return true;
    }
    if ports::all().any(|p| p.installed()) {
        return false;
    }
    REFUSED.store(true, Ordering::Relaxed);
    crate::log!("no port is installed when the game asks for its Direct3D 8: the Lua side of the plugin did not run (see MWSE.log); the game keeps MGE XE on Direct3D 9");
    true
}

/// Puts the detours on the engine functions and the recorder on the device. Every port
/// starts in mode off. A second call only puts the recorder on the device.
pub unsafe fn install() -> Result<String, String> {
    if refused() {
        return Err(refused_message());
    }
    if ports::all().any(|p| p.installed()) {
        return Ok(attach_device()?.unwrap_or_else(|| "the game has no renderer yet".to_string()));
    }
    crate::log!("nirender v{}", env!("CARGO_PKG_VERSION"));
    shadow::set_main_thread();
    shadow::calibrate_stamps();
    crate::log!("{}", crate::cores::prefer_performance_cores());
    // A panic of the Rust code ends the game. Its message and place go into the log first.
    std::panic::set_hook(Box::new(|info| {
        crate::log!("panic: {info}");
    }));
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
    // With the plugin as the Direct3D 8 of the game, every port must run as Rust: a port
    // that stays game code would call a device that draws nothing. A function that another
    // mod changed in a way the port does not follow, or a detour that failed, refuses the
    // renderer for the session, and MGE XE goes on with Direct3D 9.
    if crate::link_client::own_d3d8() {
        let cannot: Vec<&str> = ports::all().filter(|p| p.locked() || !p.installed()).map(|p| p.name).collect();
        if !cannot.is_empty() {
            REFUSED.store(true, Ordering::Relaxed);
            crate::log!("{} of {} ports cannot run as Rust: {}", cannot.len(), ports::all().count(), cannot.join(", "));
            return Err(refused_message());
        }
    }
    let attached = match attach_device()? {
        Some(text) => text,
        None => {
            // The game has no renderer yet. The recorder goes on the device at the first frame.
            if let Some(port) = ports::all().find(|p| p.addr == BEGIN_PAINT) {
                port.before.store(before_begin_paint as *const () as usize, Ordering::Relaxed);
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

static ATTACHED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Puts the recorder on the device when the game has a device and the recorder is not on
/// it. A comparison calls this first: without the recorder its Rust run makes its device
/// calls on the device of the game.
pub unsafe fn attach_device_early() {
    if !ATTACHED.load(Ordering::Relaxed) {
        if let Ok(Some(text)) = attach_device() {
            crate::log!("before the first frame: {text}");
        }
    }
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
    ATTACHED.store(true, Ordering::Relaxed);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The bytes of the game without the Morrowind Code Patch at each place where the Code
    /// Patch 2.4 (and 2.5 beta 19, which adds none) writes inside a ported function: the
    /// feature, the address and the bytes of `Morrowind.Original.exe`. The database has the
    /// bytes with each of these features on. A player can turn a feature off, and the game
    /// then has these bytes there. From `docs/scripts/mcp-overlap` of moreFPS.
    const VANILLA: &[(u32, usize, &[u8])] = &[
        // NiDX8Renderer::RenderPoints
        (11, 0x6AD84A, &[0xFF, 0x92, 0x1C, 0x01, 0x00]),
        // NiDX8Renderer::RenderLines
        (11, 0x6ADC91, &[0xFF, 0x92, 0x1C, 0x01, 0x00]),
        // NiDX8Renderer::DrawPrimitive
        (11, 0x6AEF05, &[0x4E, 0x24, 0x8B, 0x40, 0x2C, 0x57, 0x8B, 0x11, 0x55, 0x53, 0x6A, 0x00, 0x50, 0x51, 0xFF, 0x92, 0x1C, 0x01, 0x00]),
        // NiDX8Renderer::DrawSkinnedPrimitive2
        (
            11,
            0x6AF231,
            &[
                0x17, 0x33, 0xD2, 0x66, 0x8B, 0x55, 0xF4, 0x52, 0x8B, 0x54, 0x24, 0x44, 0x6A, 0x00, 0x52, 0x50, 0xFF, 0x91, 0x1C, 0x01, 0x00, 0x00, 0xEB,
                0x0C, 0x8B, 0x54, 0x24, 0x40, 0x52, 0x50, 0xFF, 0x91, 0x18, 0x01, 0x00,
            ],
        ),
        // NiDX8TexturePass::setBlendingState
        (12, 0x6B5664, &[0x18, 0x2E, 0x00]),
        // NiDX8TextureData::FindMatchingPixelFormat
        (39, 0x6B70A8, &[0x04]),
        (39, 0x6B70DF, &[0x08]),
        (39, 0x6B7357, &[0x10]),
        (39, 0x6B737A, &[0x14]),
        // NiDX8TextureStage::updateRenderer
        (131, 0x6C1BA5, &[0x37, 0xD2, 0xF9, 0xFF]),
        // NiDX8MaterialManager::UpdateToDevice
        (11, 0x6C2686, &[0x15, 0x8B, 0x44, 0x24, 0x04, 0xC6, 0x41, 0x1C, 0x00, 0x8B, 0x49, 0x18, 0x8B, 0x10, 0x51, 0x50, 0xFF, 0x92, 0xA8, 0x00, 0x00, 0x00, 0xC2, 0x04, 0x00]),
    ];

    fn containing(va: usize) -> &'static crate::gen::funcs::Func {
        crate::gen::funcs::FUNCS.iter().find(|f| f.ea <= va && va < f.ea + f.size).expect("a ported function")
    }

    /// The function as the census sees it with `bytes` written at `at`. A record of the Code
    /// Patch can start before the function or end after it (`UpdateToDevice`): the census
    /// compares the function only.
    fn live_with(func: &crate::gen::funcs::Func, at: usize, bytes: &[u8]) -> Vec<u8> {
        let mut live = census::expected(func).to_vec();
        for (i, &byte) in bytes.iter().enumerate() {
            if let Some(slot) = (at + i).checked_sub(func.ea).and_then(|offset| live.get_mut(offset)) {
                *slot = byte;
            }
        }
        live
    }

    /// True when the census follows each run of changed bytes. A byte in memory after the
    /// function (a known patch can reach there) comes from `bytes` at `at`.
    fn followed(func: &crate::gen::funcs::Func, live: &[u8], at: usize, bytes: &[u8]) -> bool {
        let runs = census::runs(live, census::expected(func));
        assert!(!runs.is_empty(), "{}: the bytes are those of the database", func.name);
        let read = |addr: usize| if (at..at + bytes.len()).contains(&addr) { bytes[addr - at] } else { live[addr - func.ea] };
        runs.iter().all(|&(offset, len)| follows_change(func, live, offset, len, read))
    }

    #[test]
    fn each_code_patch_feature_in_a_port_can_be_off() {
        for &(feature, at, bytes) in VANILLA {
            let func = containing(at);
            assert!(followed(func, &live_with(func, at, bytes), at, bytes), "feature {feature} off at {at:#x} in {}: a port does not follow it", func.name);
        }
    }

    /// Where the testing instance keeps the `mcpatch` folder of the Morrowind Code Patch (the
    /// Root folder of its mod) and `Morrowind.Original.exe` (the Root folder of overwrite). Root
    /// Builder copies them into the game folder only while the game runs.
    const MCPATCH_ROOT: &str = "D:/Modlists/Morrowind75testing/mods/Morrowind Code Patch/Root";
    const VANILLA_EXE: &str = "D:/Modlists/Morrowind75testing/overwrite/Root/Morrowind.Original.exe";
    /// The `Morrowind.exe` that the database is made from (sha256): the Code Patch has a
    /// folder of its patch file for each exe that it knows.
    const DATABASE_EXE: &str = "6fb89bbe8932b7de718c86dd4387fd59e04b9a33b011b61bc78254ded32f5456";

    /// The records of a patch file of the Code Patch: the feature, the file offset and the
    /// bytes.
    fn code_patch_records(blob: &[u8]) -> Vec<(u32, usize, &[u8])> {
        let word = |at: usize| u32::from_le_bytes([blob[at], blob[at + 1], blob[at + 2], blob[at + 3]]);
        let mut records = Vec::new();
        let mut at = 0;
        while at + 12 <= blob.len() {
            let (feature, offset, len) = (word(at), word(at + 4) as usize, word(at + 8) as usize);
            records.push((feature, offset, &blob[at + 12..at + 12 + len]));
            at += 12 + len;
        }
        records
    }

    /// Step 0b.4 of the wgpu plan: a new version of the Morrowind Code Patch that writes inside
    /// a ported function fails here, on the machine of a developer, and not in the session of
    /// a player. Each record of each patch file in `NIRENDER_MCPATCH` (folders with a folder
    /// `mcpatch`, with `;` between them; `MCPATCH_ROOT` without it) that writes inside a ported
    /// function must be in `ports::CODE_PATCH_RECORDS`, and the census must follow it with the
    /// option on (the bytes of the record) and off (the bytes of `NIRENDER_VANILLA_EXE`, or
    /// `VANILLA_EXE`). Skipped without the files: the patch file is not ours to ship.
    #[test]
    fn every_code_patch_record_in_a_port_is_followed() {
        let roots: Vec<String> =
            std::env::var("NIRENDER_MCPATCH").map(|roots| roots.split(';').map(str::to_string).collect()).unwrap_or_else(|_| vec![MCPATCH_ROOT.to_string()]);
        let vanilla_path = std::env::var("NIRENDER_VANILLA_EXE").unwrap_or_else(|_| VANILLA_EXE.to_string());
        let Ok(vanilla) = std::fs::read(&vanilla_path) else {
            eprintln!("skipped: no {vanilla_path}");
            return;
        };
        let mut checked = 0;
        for root in &roots {
            let path = format!("{root}/mcpatch/{DATABASE_EXE}/patch");
            let Ok(blob) = std::fs::read(&path) else {
                eprintln!("no {path}");
                continue;
            };
            for (feature, offset, data) in code_patch_records(&blob) {
                let va = 0x400000 + offset;
                for port in ports::all() {
                    let func = crate::gen::funcs::FUNCS.iter().find(|f| f.ea == port.addr).expect("a port is in the function table");
                    let (lo, hi) = (va.max(func.ea), (va + data.len()).min(func.ea + func.size));
                    if lo >= hi {
                        continue;
                    }
                    let known = ports::CODE_PATCH_RECORDS.iter().any(|&(f, start, len)| f == feature && start <= lo && hi <= start + len);
                    assert!(known, "{path}: feature {feature} writes {lo:#x}..{hi:#x} in {}, and ports::CODE_PATCH_RECORDS has no such record", func.name);
                    for (state, bytes) in [("on", data), ("off", &vanilla[offset..offset + data.len()])] {
                        let live = live_with(func, va, bytes);
                        if census::runs(&live, census::expected(func)).is_empty() {
                            continue;
                        }
                        assert!(followed(func, &live, va, bytes), "{path}: feature {feature} {state} at {lo:#x} in {}: a port does not follow it", func.name);
                    }
                    checked += 1;
                }
            }
        }
        eprintln!("{checked} records of the Code Patch in ports checked, on and off");
    }

    #[test]
    fn a_change_of_another_kind_is_not_followed() {
        // The places of feature 11 with other bytes: a mod that is not known. The other
        // features change the target of a call (12, 131), which a port calls whatever it is,
        // or an operand (39), which a port reads whatever it is.
        for &(feature, at, bytes) in VANILLA.iter().filter(|(feature, _, _)| *feature == 11) {
            let func = containing(at);
            let other: Vec<u8> = bytes.iter().map(|b| b ^ 0x5A).collect();
            assert!(!followed(func, &live_with(func, at, &other), at, &other), "feature {feature} at {at:#x}: other bytes are followed");
        }
    }
}
